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

The design lives in the code, in module documentation next to the thing it explains. One file carries
the history instead: [REVIEW.md](REVIEW.md), a critical scan of the crate, with what each finding
turned into.

Everything the review found is implemented here -- including a worked packet set and server flow in
[`src/demo/`](src/demo) -- so it can be judged by running `cargo test -p passage-driver` rather than
by reading prose alone. Where a finding was deliberately not acted on, `REVIEW.md` says so and why.

## Static, and per connection

Two halves, and the vocabulary follows the split: a `Router` is built once at startup and shared,
a `Connection` is created for each accepted socket and owns everything mutable.

| Static, built once  | One per accepted socket                          |
|---------------------|--------------------------------------------------|
| `Router`            | `Connection`                                     |
| `ConnectionConfig`  | `ConnectionHandle`                               |
| the handlers        | the state `S`, and a `Ctx` per handler call      |
| the dispatch tables | a `RouterDispatcher`, bound to the table for the negotiated version |

The two never meet directly: a `Connection` depends on the `Dispatcher` trait, which `conn`
declares and `router` implements. So the connection holds no table and resolves no packet ID, and
it can be driven by a test double instead -- see [`tests/dispatch.rs`](tests/dispatch.rs).

`server::Server` is the bridge, in the shape Axum uses -- with one difference: state here is *per
connection*, so it takes a factory rather than a value and calls it once per socket.

```rust
let listener = TcpListener::bind("0.0.0.0:25565").await?;

Server::builder()
    .listener(listener)
    .dispatch(Arc::new(router()?))
    .state(|addr| Session { peer: Some(*addr), ..Session::default() })
    .tick_interval(Duration::from_secs(16))
    .max_connections(10_000)
    .graceful_shutdown(shutdown)
    .await;
```

Everything is built the same way -- `Router::builder()`, `Server::builder()`,
`Connection::builder()`. The three a server cannot do without are type parameters that start unset,
so `.await` does not exist until a listener, something to dispatch to and a state factory have all
been given: "you cannot forget one" survives the move away from positional arguments.
`serve(listener, dispatch, state)` is the same three, positionally, for when naming them adds
nothing.

One connection without the accept loop is `Connection::builder`, which is what the server uses per
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

## Each layer keeps its own books

Four things can turn a peer away, and none of them tells the others:

| Layer          | Decides                       | Still holding                            |
|----------------|-------------------------------|------------------------------------------|
| a `Layer`      | not this peer, not like this  | the TLS error, the bucket, the ban list  |
| a handler      | this *session* is refused     | the packet, the phase, the session state |
| the connection | it ended, and how             | the `Ending`, the state, the version     |
| the driver     | what to record about all that | nothing else -- and it records only its own |

Each one holds, at the moment it decides, everything a log line or a metric about that decision
could want -- so each one records its own. A `Layer` that refuses returns `None` and nothing more;
`&self` is what lets it be a named type with a counter in it. A handler that refuses a login logs
the player and the reason, which it is holding and nobody else is.

The driver's share is deliberately small: a `connection` span, and one event when a connection ends
saying how long it took, which `Ending` it had and which version and phase it reached -- at `warn`
if the cause was ours, `debug` otherwise. Everything a handler logs lands inside that span, so the
correlation a central reporter used to provide comes from the span instead.

There is no hook told about every possible fate, and that is the point. It sounds like
consolidation and is the opposite: every layer has to flatten what it knows into a vocabulary the
driver invented for it, the hook grows an arm per layer, and the place that ends up knowing
everything is the place furthest from where any of it happened. That is the same trade the driver
refuses one level down -- there is no `Ending::Refused`, because the handler that refused a login
knows it did.

## Everything before the protocol is a layer

A socket usually needs something done to it before the protocol starts. All of it is one shape --
take the socket and the address, hand back a socket and an address, or refuse:

```rust
Server::builder()
    .listener(listener)
    .layer(ProxyProtocol::new(trusted))   // rewrites the address
    .layer(Tls::new(acceptor))            // changes the socket type
    .layer(rate_limiter)                  // refuses, and counts its own refusals
    .dispatch(router)
    .state(session)
```

Layers run in the order written, each seeing what the one before produced, all on the connection's
own task -- so a peer that connects and says nothing holds up nobody else. A `Listener` is therefore
only a *source* of sockets: one associated type for the socket, one for the address, one method.

The driver ships no layers. A PROXY implementation belongs next to the PROXY parser and reaches the
builder as an extension trait over `Server`, which is why nothing in the accept loop mentions
proxies, TLS or rate limits.

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
