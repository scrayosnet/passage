# 4. The runtime: driving one connection

The driver's whole job: frame, dispatch, tick, shut down. This document is about the concurrency
model, because that is where the bugs that only appear under load live.

## D. The loop

### D1 -- Sequential `&mut self` (today)

`Connection::listen` owns the socket and awaits packets inline.

| Pros                                             | Cons                                                                     |
|--------------------------------------------------|--------------------------------------------------------------------------|
| Simplest possible ordering guarantees             | The flow *is* the loop, so it cannot be extended without editing it       |
| No channels, no locks, no `Arc`                    | Concurrent work needs an ad-hoc `tokio::spawn` plus a `select!` arm       |
| Cancellation safety is easy to see                 | Ticks have to be woven into the middle of the flow                        |

### D2 -- One task, ordered operation queue *(recommended)*

The driver owns the socket; handlers queue operations.

```rust
let step = tokio::select! {
    biased;
    op       = self.ops.recv()                               => Step::Op(op),
    result   = async { self.pending.as_mut().expect("set").await }, if self.pending.is_some()
                                                             => Step::Pending(result),
    joined   = self.detached.join_next(), if !self.detached.is_empty()
                                                             => Step::Detached(joined),
    ()       = self.shutdown.cancelled()                     => Step::Shutdown,
    _        = async { self.ticker.as_mut().expect("set").tick().await },
               if self.ticker.is_some() && self.pending.is_none()
                                                             => Step::Tick,
    frame    = self.framed.next(), if self.pending.is_none()  => Step::Frame(frame),
};
```

The priority order is the design:

1. **Operations first.** A handler's writes reach the socket before the next packet is looked at.
2. **The pending handler second.** At most one handler per connection runs at a time.
3. **Input last, and only when nothing is pending.** This is the backpressure: a slow handler stops
   us from buffering more input, and no packet can be dispatched into a half-finished transition.

| Pros                                                                              | Cons                                                                          |
|-----------------------------------------------------------------------------------|-------------------------------------------------------------------------------|
| One writer, so no lock on the socket and no interleaved frames                      | Writes cost a channel hop (uncontended, but not free)                          |
| Side effects are ordered relative to each other -- see the encryption note below     | Reads and writes are not concurrent: a blocked write stalls reads              |
| Handlers can be pure functions; the loop is testable on a `duplex` pair             | Handlers cannot hold state across an await, by construction (see `Update`)      |
| Detached work is opt-in and visible                                                 | The `select!` needs a `Step` enum to avoid borrowing `self` twice               |

### D3 -- Split read and write halves into two tasks

| Pros                                                                     | Cons                                                                                        |
|--------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| Full duplex: a stalled write does not stop reads                          | The encryption switchover now spans two tasks, and "enable after this write, before that read" becomes a synchronisation problem |
| The write half can coalesce packets into one syscall                       | Two tasks per connection instead of one                                                      |
| AES-CFB8 has separate encrypt/decrypt state, so the cipher splits cleanly  | Ordering bugs become timing-dependent, i.e. the worst kind                                   |

**Recommendation: D2.** Passage's traffic per connection is a handful of small packets, so duplex
throughput is not the constraint. The `Cipher` trait keeps the door open: it is already two
independent methods, so a future split does not touch the framing code.

## Why `Flow` instead of `async fn` everywhere

```rust
pub enum Flow<T> {
    Ready(T),
    Pending(BoxFuture<'static, T>),
}
```

| Approach                              | Cost per packet                                   | Notes                                                        |
|---------------------------------------|---------------------------------------------------|--------------------------------------------------------------|
| `async fn` in trait                    | A generated state machine; boxed to be dyn-safe    | Uniform, but every trivial handler allocates                  |
| `Box<dyn Future>` always               | One allocation + one virtual call                  | Same cost, no upside                                          |
| `Flow` *(chosen)*                      | Nothing for sync handlers; one box for async ones  | The driver can *see* whether to pause input -- the key benefit |

The performance argument is secondary. The real reason is that `Flow` makes "this handler is going to
wait" visible to the driver, which is what lets it pause reads and keep ordering. An `async fn` that
completes immediately is indistinguishable from one that waits a second.

Note the `'static` bound on the future: an async handler cannot borrow session state. That is
deliberate, and the reason the next section exists.

## State access

### Option 1 -- `Arc<Mutex<S>>`

What `src/hooks.rs` currently sketches (`state: Arc<Mutex<S>>`).

| Pros                                | Cons                                                                                          |
|-------------------------------------|-----------------------------------------------------------------------------------------------|
| Async handlers can just lock it      | A lock held across an await stalls ticks and every other handler for that connection            |
| Familiar                            | Atomic traffic and an allocation per connection for state that has exactly one writer           |
|                                     | Deadlock is expressible (handler locks, awaits something that locks)                            |

### Option 2 -- `&mut S` for sync handlers, deferred `Update` for async ones *(recommended)*

```rust
pub struct Ctx<'a, S> {
    pub state: &'a mut S,          // exclusive, no lock
    pub conn: &'a ConnHandle<S>,   // cheap clone, safe across awaits
}

// async handlers hand a change back instead of holding the state
Ok(Update::apply(move |session: &mut Session| session.profile = Some((name, id))))
```

The driver applies the update with exclusive access at a defined point: after the handler resolves
and before the next frame is read.

| Pros                                                                | Cons                                                              |
|---------------------------------------------------------------------|-------------------------------------------------------------------|
| Sync handlers (the majority) are lock-free and atomic-free            | Async handlers write state in a closure, which reads indirectly    |
| A lock cannot be held across an await, because there is no lock        | One boxed closure per async handler that changes state             |
| The point at which state changes is defined, not "whenever we got it"  | `Update` cannot *read* state; it has to be given what it needs     |

The same mechanism covers detached work: `ConnHandle::detach` takes a future returning
`Result<Update<S>>`, so backend selection can run alongside keep-alives and still write its result
back safely.

## The ordering hazards this model exists to remove

### 1. Concurrently spawned handlers

`src/driver.rs` currently spawns every pending hook into a `JoinSet`:

```rust
Flow::Pending(future) => {
    self.pending_hooks.spawn(future);   // and then keep reading...
    Ok(())
}
```

Failure scenario: a client pipelines `LoginStart` and `LoginAcknowledged` in one TCP segment. Both
handlers start; the acknowledgement handler runs against a session whose authentication has not
finished, and its `Transfer` may be written before `LoginSuccess`. It will pass every test on
localhost and fail on a real network. Holding a *single* pending slot and pausing reads removes the
class of bug rather than the instance.

### 2. The encryption switchover

Every byte before the switch must be plaintext and every byte after must be ciphertext, on both
directions. If enabling encryption were a direct call on the codec, a packet queued but not yet
written would be encrypted retroactively. Routing it through the same ordered queue as sends makes
the position in the stream explicit:

```rust
ctx.send(&EncryptionRequest { /* ... */ })?;   // plaintext
ctx.conn.encrypt(cipher)?;                     // everything after this is encrypted
```

### 3. Phase transitions

A phase change alters how the *next* inbound frame is decoded. The driver reads `phase` per frame,
and the queue is drained before a frame is read, so a transition cannot be applied halfway through a
batch of already-decoded packets. The invariant that remains, documented on `set_phase`: a phase
change must be triggered by the peer's transition packet, or be preceded by `flush`.

## Ticks, deadlines and shutdown

* **Ticks** are the driver's timer, not the flow's. The keep-alive policy is one handler
  (`on_tick`), and it is skipped while a handler is pending -- a keep-alive queued behind a stalled
  handler is worse than a late one.
* **Shutdown** is a `CancellationToken` shared with detached work. `Driver::run` returns
  `Completion::Cancelled`, then cancels detached tasks and closes the socket. The token is also
  reachable from `ConnHandle::shutdown()`, so a detached adapter call can observe it.
* **Deadlines** are not implemented. The right shape is a per-phase budget in `DriverConfig`
  (`max_time_in_phase`, `max_packets_in_phase`), enforced in the loop, because a connection that
  sits in the login phase forever is the cheapest denial of service there is. See
  [05-errors-and-hardening.md](05-errors-and-hardening.md).

## Known limitation: the operation queue is unbounded

`ConnHandle` uses `mpsc::unbounded_channel`. Since the driver drains operations at the highest
priority, a backlog only forms when the socket itself blocks -- i.e. against a slow client. A handler
that queues in a loop against such a client grows memory without limit.

The fix is a bounded channel with an explicit policy: `try_send` for synchronous handlers (failing
with a `Backpressure` error) and `send().await` for asynchronous ones. It is left out of the
reference implementation deliberately -- it changes `ConnHandle::send` from infallible-modulo-closed
to a fallible operation that handlers must think about, and that is a decision worth making
explicitly rather than by default.
