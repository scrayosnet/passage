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

The driver owns the socket, the state, the phase and the version. Handlers queue operations.

```rust
let step = tokio::select! {
    biased;

    // 1. Everything handlers asked for, before anything else.
    op    = self.ops.recv()  => Step::Op(op.expect("the driver holds a handle")),

    // 2. Finished handler tasks, exclusive or not -- one set, one arm.
    done  = self.tasks.next(), if !self.tasks.is_empty() => Step::Task(done),

    // 3. Cancellation.
    ()    = self.shutdown.cancelled() => Step::Shutdown,

    // 4. Deadlines.
    ()    = expire(&mut self.lifetime), if self.lifetime.is_some() => Step::Expired,
    ()    = expire(&mut self.idle),     if self.idle.is_some()     => Step::Expired,

    // 5. Ticks -- but never while the peer must stay quiet.
    ()    = tick(&mut self.ticker),
            if self.ticker.is_some() && self.exclusive == 0 => Step::Tick,

    // 6. Input.
    frame = self.framed.next() => Step::Frame(frame),
};
```

The priority order is the design:

1. **Operations first.** *All* of a handler's effects -- the packets it queued, the phase it moved
   to, the version it pinned, the state it changed, the task it started -- are applied before the
   next packet is looked at. That is what makes an operation queue equivalent to exclusive access
   without a lock.
2. **Tasks second**, so a finished handler's result is folded in before more input arrives.
3. **Input last.** Not conditional on anything: see [the read gate](#the-read-gate).

| Pros                                                                              | Cons                                                                          |
|-----------------------------------------------------------------------------------|-------------------------------------------------------------------------------|
| One writer, so no lock on the socket and no interleaved frames                      | Writes cost a channel hop (uncontended, but not free)                          |
| Side effects are ordered relative to each other -- see the hazards below             | Reads and writes are not concurrent: a blocked write stalls reads              |
| Handlers are pure functions of `(&S, packet)`; the loop is testable on a `duplex` pair | A handler cannot observe its own effects                                    |
| Concurrency is opt-in and visible                                                   | The `select!` needs a `Step` enum to avoid borrowing `self` twice               |

### D3 -- Split read and write halves into two tasks

| Pros                                                                     | Cons                                                                                        |
|--------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| Full duplex: a stalled write does not stop reads                          | The encryption switchover now spans two tasks, and "enable after this write, before that read" becomes a synchronisation problem |
| The write half can coalesce packets into one syscall                       | Two tasks per connection instead of one                                                      |
| AES-CFB8 has separate encrypt/decrypt state, so the cipher splits cleanly  | Ordering bugs become timing-dependent, i.e. the worst kind                                   |

**Recommendation: D2.** Passage's traffic per connection is a handful of small packets, so duplex
throughput is not the constraint. The coalescing advantage of D3 is available without it: the driver
`feed`s each `Op::Send` and flushes once the operation queue is empty, so a handler that queues three
packets produces one write. The `Cipher` trait keeps the door open -- it is already two independent
methods, so a future split does not touch the framing code.

## Everything a handler does is an operation

```rust
pub enum Op<S> {
    Send(Box<dyn AnyPacket>),
    Encrypt(Box<dyn Cipher>),
    SetVersion(ProtocolVersion),
    SetPhase(Phase),
    With(Box<dyn FnOnce(&mut S) + Send>),
    Spawn { future: BoxFuture<'static, Result<()>>, exclusive: bool },
    Flush(oneshot::Sender<()>),
    Close,
}
```

That is the entire vocabulary. A handler receives a read-only view and returns `Result<()>`:

```rust
pub struct Ctx<'a, S> {
    pub state: &'a S,              // read-only; changes go through `update`
    pub conn: &'a ConnHandle<S>,   // cheap clone, safe to hold across awaits
    // plus `limits()`, `version()`, `phase()` -- values, not atomics
}
```

Three properties follow, and none of them needs a lock:

* **One writer.** Nothing but the driver touches the socket, the state, the phase or the version, so
  nothing can interleave -- not even a packet queued from a background task.
* **No stale reads.** `ctx.version()` and `ctx.phase()` are values the driver passed in. An earlier
  design kept them in atomics on the shared handle, which meant a handler could read a phase another
  task had already moved past, and the type had to document that.
* **No half-applied handlers.** A handler that queues two operations and then returns an error has
  applied neither. Under `&mut S` it would have applied the first.

The cost is that a handler cannot observe its own effects: `ctx.send(..)` followed by `ctx.state` still
shows the old state. In exchange, "record the profile, then announce it" is expressible, which is what
the next section is about.

### Why sending is an operation rather than an encode-then-queue

`Op::Send` carries the packet, not bytes. The driver encodes it when it drains the operation, using
its own version. So a handler never needs to know the protocol version to send something, and a task
that outlives its handler cannot encode against a stale one. The ordering property that matters --
bytes hit the socket in operation order, not in whatever order handlers finished encoding -- is
preserved either way, because the encode happens *at* drain time.

## State access

### Option 1 -- `Arc<Mutex<S>>`

What the first sketch of this crate had (`state: Arc<Mutex<S>>`).

| Pros                                | Cons                                                                                          |
|-------------------------------------|-----------------------------------------------------------------------------------------------|
| Async handlers can just lock it      | A lock held across an await stalls ticks and every other handler for that connection            |
| Familiar                            | Atomic traffic and an allocation per connection for state that has exactly one writer           |
|                                     | Deadlock is expressible (handler locks, awaits something that locks)                            |

### Option 2 -- `&mut S` for sync handlers, a deferred `Update` for async ones

An intermediate design: `Ctx` carried `&mut S`, and an async handler returned an `Update<S>` closure
that the driver applied once the future resolved.

| Pros                                                          | Cons                                                                       |
|---------------------------------------------------------------|----------------------------------------------------------------------------|
| Sync handlers are lock-free and atomic-free                    | **One change, at the end.** A task cannot commit progress as it goes        |
| No lock can be held across an await, because there is no lock  | `Update` cannot *read* state; it has to be given everything it needs        |
|                                                               | Packets are sent *during* the future, state lands *after* it -- see below   |

The last row is the one that killed it. A task queues its packets as it goes but hands its state
change back only at the end, so anything running in between sees a session whose outcome has been
announced to the client but not recorded locally. The tick handler is exactly such a thing.

### Option 3 -- Reads through `&S`, writes as an operation *(recommended)*

```rust
// fire and forget, ordered with everything else queued
conn.update(|session: &mut Session| session.profile = Some(profile))?;

// awaits application, and can read back -- so it doubles as a barrier
let host = conn.with(|session: &mut Session| session.host.clone()).await?;
```

Both are the same operation. `update` is the common case; `with` carries a oneshot so a background
task can read state it does not own -- something `Update` could not do at all.

| Pros                                                                     | Cons                                                             |
|--------------------------------------------------------------------------|------------------------------------------------------------------|
| A state change is ordered against the packets around it                   | One boxed closure per change                                     |
| A task can commit progress incrementally, not once at the end             | The closure runs on the driver, so it must not block or await     |
| Background tasks can read state, not only write it                        | `ctx.state` is a snapshot: a handler's own change is not visible  |
| Removes `Update<S>` from every future's return type                       |                                                                  |

Two rules, both consequences of the closure running inside the drain loop with `&mut S`:

* It must not block and cannot await. It *may* queue further operations through a cloned handle.
* If the connection ends first, the oneshot is dropped and `with` resolves to `Error::Closed` -- the
  same contract `flush` has.

## The read gate

`ctx.exclusive(future)` marks work the peer is expected to wait for: an authentication call, a
session-server round trip, a cookie lookup. `ctx.spawn(future)` is the same thing without that claim.

The predecessor of this was `Flow::Pending`, which made the driver *stop reading* until the future
resolved. That framing was wrong in a way worth naming: it is not backpressure. While Passage waits
for authentication a compliant client sends nothing, so "stop reading" was a **protocol assertion
dressed up as flow control** -- and an unenforced one, because an early packet simply sat in the
socket buffer and was processed afterwards as though it had arrived on time.

So the gate keeps polling the socket and treats what it finds as a protocol break:

```rust
fn handle_frame(&mut self, frame: Frame) -> Result<()> {
    self.reset_idle();
    if self.exclusive > 0 {
        return Err(ProtocolError::EarlyPacket { phase: self.phase, id: frame.id }.into());
    }
    // ...
}
```

Two gains over pausing reads:

* **An early packet is reported.** `EarlyPacket` is a `Peer` error with its own metric label, instead
  of a packet replayed into a session that was not ready for it.
* **A hangup is noticed at once.** Previously, a client that disconnected mid-authentication left the
  adapter call running and the connection slot held until the future resolved on its own. Now the EOF
  arrives on the same arm as everything else.

`Framed::next()` is cancel-safe -- the partial frame lives in the codec, not in the future -- so
leaving this arm enabled on every iteration costs nothing.

### Why exclusivity is a property of the task, not a flag

`exclusive` is a counter the *driver* maintains: incremented when it drains an `Op::Spawn { exclusive:
true }`, decremented when that task resolves. A manual "stop accepting packets" flag would do the same
job and can leak -- whoever shuts it must remember to open it, and a missed reopen stalls the
connection. Tying it to the task removes the failure mode rather than documenting it.

Its soundness rests entirely on the operation queue's priority. A handler returns, its `Op::Spawn` is
drained *before* the read arm is reached, so `exclusive` is already non-zero by the time the next
frame -- buffered or freshly arrived -- is considered.

Ticks are suppressed while `exclusive > 0`, for a reason specific to this model: a keep-alive sent
into a gated window would invite exactly the response the gate rejects.

### The strictness trade

A client that pipelines during the exclusive window is now disconnected where it used to be tolerated.
Vanilla waits at every point where Passage needs it to (`EncryptionResponse` → verify →
`LoginSuccess` → `LoginAcknowledged`), so this should be safe, but it is a behaviour change against
unknown clients. If it ever needs a fallback, "defer instead of reject" is a config field and a select
condition rather than a second code path:

```rust
frame = self.framed.next(), if self.exclusive == 0 || self.config.reject_early => ...
```

### Why not `JoinSet`

Tasks are polled on the connection's own task, in a `FuturesUnordered`. That removes `tokio::spawn`,
the `JoinError` handling, and the panic-to-internal-error conversion; cancellation becomes dropping
the set rather than an awaited `shutdown()`. The trade is that a panicking handler takes the
connection task down instead of being caught -- which whoever spawned the connection sees anyway.
Switch back to `JoinSet` if per-task panic isolation is worth the extra arm; nothing else in this
design changes.

## The ordering hazards this model exists to remove

### 1. Concurrently spawned handlers

An earlier version of `src/driver.rs` spawned every pending hook into a `JoinSet` and kept reading:

```rust
Flow::Pending(future) => {
    self.pending_hooks.spawn(future);   // and then keep reading...
    Ok(())
}
```

Failure scenario: a client pipelines `LoginStart` and `LoginAcknowledged` in one TCP segment. Both
handlers start; the acknowledgement handler runs against a session whose authentication has not
finished, and its `Transfer` may be written before `LoginSuccess`. It will pass every test on
localhost and fail on a real network. The gate removes the class of bug rather than the instance --
and, unlike pausing reads, it also tells you the client did it.

### 2. The encryption switchover

Every byte before the switch must be plaintext and every byte after must be ciphertext, in both
directions. Routing it through the same ordered queue as sends makes the position in the stream
explicit:

```rust
ctx.send(EncryptionRequest { /* ... */ })?;   // plaintext
ctx.encrypt(cipher)?;                         // everything after this is encrypted
```

The codec only encrypts what it encodes from that point on, so bytes already sitting in the write
buffer stay plaintext and no flush is needed to get the switchover point right.

### 3. Phase transitions

A phase change alters how the *next* inbound frame is decoded, and it is the one piece of connection
state that changes repeatedly, mid-stream. As an operation it lands exactly between the packet queued
before it and the one queued after, so "send the last packet of this phase, then switch" is
expressible and the peer's next frame is decoded against the table the handler intended.

The earlier design applied it eagerly through an atomic, which is why `set_phase` carried an
invariant comment telling callers when they were allowed to call it. The invariant is now structural:
the queue is drained before any frame is read, so there is no window in which it could be wrong.

### 4. State versus wire order

Covered under [state access](#state-access) as the defect that retired option 2: `update` before
`send` means the state is committed before the packet that announces it, and `send` before `update`
means the opposite. Both are now sayable, and which one you get is whichever you wrote. Proven by
`tests/flow.rs::state_is_recorded_before_the_packet_that_announces_it`.

## Ticks, deadlines and shutdown

* **Ticks** are the driver's timer, not the flow's. The keep-alive policy is one handler (`on_tick`),
  and the timer is only armed if the router actually has one -- otherwise it would wake the task up to
  do nothing.
* **Deadlines** are the driver's job for two reasons: it owns the clock and the socket, and wrapping
  `driver.run()` in `tokio::time::timeout` drops the future mid-flight, so the shutdown path never
  runs and in-flight tasks are not cancelled cleanly.

  ```rust
  pub struct DriverConfig {
      pub max_lifetime: Option<Duration>,   // hard cap on the whole connection
      pub max_idle: Option<Duration>,       // reset on every frame the peer sends
      // ...
  }
  ```

  Both end the connection as `Completion::TimedOut`, which is a completion and not an error. A
  connection that sits in the login phase forever is the cheapest denial of service there is, and
  `max_lifetime` is also the backstop for an exclusive task that never resolves -- the one way the
  read gate could otherwise stall a connection indefinitely.
* **Shutdown** is a `CancellationToken` shared with background work. `Driver::run` returns
  `Completion::Cancelled`, then drops the tasks and closes the socket. The token is also reachable
  from `ConnHandle::shutdown()`, so a long adapter call can observe it.

Per-*phase* budgets (`max_time_in_phase`, `max_packets_in_phase`) are still open; see
[05-errors-and-hardening.md](05-errors-and-hardening.md). The two global deadlines cover the denial
of service case, and phase budgets are a policy that belongs to the server layer.

## Known limitation: the operation queue is unbounded

`ConnHandle` uses `mpsc::unbounded_channel`. Since the driver drains operations at the highest
priority, a backlog only forms when the socket itself blocks -- i.e. against a slow client. A handler
that queues in a loop against such a client grows memory without limit.

The fix is a bounded channel with an explicit policy: `try_send` for synchronous handlers (failing
with a `Backpressure` error) and `send().await` for asynchronous ones. It is left out of the
reference implementation deliberately -- it changes every `ConnHandle` method from
infallible-modulo-closed to something handlers must think about, and that is a decision worth making
explicitly rather than by default. `max_lifetime` bounds the damage in the meantime.
