# Passage Driver

This package is a sketch for the Passage Driver, the next-gen Passage implementation. It provides a
general-purpose backbone for handling the Minecraft protocol. Instead of handling packets stricty
sequentially in one big method, it uses hooks and handlers to implement the custom protocol logic.
The backbone only provides the packet parsing and error handling.

> The crate contains a working implementation of the design, including a worked packet set and
> server flow, so it can be judged by running it. It is a proposal, not a commitment.

## Requirements

Given that this is a next-gen implementation, it should try to be as efficient as possible. It should
also fix issues with the current implementation.
- Packets should not hardcode the packet ID and codec.
  - Instead, they should be able to react to the current protocol version.
  - Passage should be backward-compatible with all protocol versions without duplicating the code.
    - Changes include packet ID changes, adding or removing packets, adding or removing fields from packets
  - Do not create a new packet type for every protocol version, instead use optional fields or similar.
- The backbone (currently called "driver") should only implement the necessary functions (e.g., parsing, error handling, ticks for keep alive packets, and shutdown)
  - This should follow state-of-the-art frameworks like Ktor, Axum, or Tonic
- Ontop of the backbone, a basic server and client implementation should be provided.
  - It uses hooks of its own to implement and overwrite logic
- Ontop of the server, the Passage router with its adapter can be implemented (or configured).
- The application should keep the telemetry and even expand upon it by having better traces

## How the design is recorded

The design lives in the code, in module documentation next to the thing it explains. Two files carry
the history instead:

| Document                                       | What it holds                                             |
|------------------------------------------------|-----------------------------------------------------------|
| [REVIEW.md](REVIEW.md)                         | A critical scan of this crate against the implementation it replaces, with what was reproduced and how |
| [REVIEW_DECISIONS.md](REVIEW_DECISIONS.md)     | What was decided about each finding, and how it was solved |

Everything the review found and the decisions accepted is implemented here -- including a worked
packet set and server flow in [`src/demo/`](src/demo) -- so it can be judged by running
`cargo test -p passage-driver` rather than by reading prose alone. Where a decision was deferred
rather than applied, `REVIEW_DECISIONS.md` says so and why.

## Static, and per connection

Two halves, and the vocabulary follows the split: a `Router` is built once at startup and shared,
a `Connection` is created for each accepted socket and owns everything mutable.

| Static, built once  | One per accepted socket                          |
|---------------------|--------------------------------------------------|
| `Router`            | `Connection`                                     |
| `ConnectionConfig`  | `ConnectionHandle`                               |
| the handlers        | the state `S`, and a `Ctx` per handler call      |
| the dispatch tables | a `RouterDispatcher`, holding the table for the negotiated version |

The two never meet directly: a `Connection` depends on the `Dispatcher` trait, which `conn`
declares and `router` implements. So the connection holds no table and resolves no packet ID, and
it can be driven by a test double instead -- see [`tests/dispatch.rs`](tests/dispatch.rs).

`server::serve` is the bridge, in the shape Axum uses -- with one difference: state here is *per
connection*, so it takes a factory rather than a value and calls it once per socket.

```rust
let listener = TcpListener::bind("0.0.0.0:25565").await?;

serve(listener, Arc::new(router()?), |addr| Session { peer: Some(*addr), ..Session::default() })
    .config(config)
    .max_connections(10_000)
    .with_graceful_shutdown(shutdown)
    .on_finish(log_completion)
    .await;
```

One connection without the accept loop is `Connection::new`, which is what `serve` calls per
socket and what the tests drive over a socket pair.

## Versions are read, not listed

A packet declares the IDs it has had, newest first, and nothing else names a protocol version:

```rust
const IDS: &[(ProtocolVersion, i32)] = &[(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)];
```

Because that table is data, the router reads the *thresholds* out of every registered packet and
builds one dispatch table per version at which dispatch actually changes. There is no list of
supported versions, which means there is none to leave a release out of: 1.21.2 is served exactly
like 1.21.1 because nothing between them differs.

A version no threshold can place -- a snapshot, which sets bit 30 and so compares above every
release, or a negative number a client is free to send -- is put on the floor instead, by one method
that both directions use. So such a peer is still *answerable*: it gets a status response, which is
how it learns which version to install, and a reason if it tries to log in. What it does not get is
a version-gated field written for a client that cannot read it.

## Ending is something you can answer

A connection ends in one of two ways, and the difference is **who decided** -- which is exactly the
`Result` it reports. `Ok(())` means a handler closed it and there is nothing left to do.
`Err(Ending)` is everything else: the peer hung up, a deadline expired, a shutdown arrived, or
something failed. All four reach the dispatcher's `on_error`, with everything already queued still
on its way out:

```rust
fn on_error(ctx: Ctx<'_, Session>, ending: &Ending) -> Result<()> {
    let (label, reason) = reason_for(ending);
    ctx.batch(|batch| {
        batch.send(LoginDisconnect::text(reason))?;
        batch.update(move |session: &mut Session| session.refused = Some(label));
        batch.close();
        Ok(())
    })
}
```

A batch reaches the connection with nothing interleaved, and queues nothing at all if building it
fails. That is what a disconnect needs: the message, the record and the close are one act, and a
message that cannot be encoded must not leave part of one behind.

A hangup belongs on the `Err` side even though nobody did anything wrong. The question the split
answers is not "was this a failure" -- `Ending::error()` answers that, and says no for three of the
four -- but "did we finish what we were doing". A client that disappears while its backend is being
selected has left a selection running, and releasing it is the same job as releasing it after a
timeout. `on_error` is the one place that job can live, and it runs for every ending we did not
choose.

Note what the driver is *not* asked to remember. There is one `Completion::Closed`, not a second
variant for "we refused them" -- the handler that refused knows it did, and writes the reason into
its own state, which comes back in the connection's `Outcome`. A completion variant could only have
carried the bare fact; a field carries the reason with it, and that is what a metric wanted anyway.
Recovering from a failure is the same argument in the other direction: `on_error` gets the last word
on the wire, but whether an error is survivable at all is decided by the handler that raised it,
where the packet and the state are still in hand.

## Shape of a handler

Every effect a handler has is an operation queued on the connection, which owns the socket, the
connection state, the phase and the protocol version:

```rust
fn on_login_acknowledged(ctx: Ctx<'_, Session>, _packet: LoginAcknowledged) -> Result<()> {
    ctx.set_phase(Phase::Configuration)?;

    let conn = ctx.conn.clone();
    // `spawn` keeps dispatch running (keep-alives must keep flowing); `exclusive` would require
    // the peer to stay quiet until the task resolves.
    ctx.spawn(async move {
        let (host, port) = select_backend().await?;
        conn.send(Transfer { host, port })?;
        conn.close()
    })
}
```
