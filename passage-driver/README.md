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

## Design proposals

[`docs/`](docs/README.md) works these requirements into concrete proposals -- one document per
decision, each with the options, their trade-offs and a recommendation:

| Document                                                        | Decision                                              |
|-----------------------------------------------------------------|-------------------------------------------------------|
| [01-problem-analysis.md](docs/01-problem-analysis.md)           | What the current implementation gets wrong, and why   |
| [02-versioning.md](docs/02-versioning.md)                       | How packets carry protocol-version differences        |
| [03-dispatch.md](docs/03-dispatch.md)                           | How packets reach protocol logic                      |
| [04-runtime.md](docs/04-runtime.md)                             | How a connection is driven, ordered and backpressured |
| [05-errors-and-hardening.md](docs/05-errors-and-hardening.md)   | Error taxonomy, limits, and the no-panic rules        |
| [06-layering-and-telemetry.md](docs/06-layering-and-telemetry.md)| Crate layering, adapters, tracing and metrics        |
| [07-reference-implementation.md](docs/07-reference-implementation.md) | The working code in `src/`, and what it proves    |
| [08-refinements.md](docs/08-refinements.md)                     | The review of the first implementation, and what changed |

The recommended option of every proposal is implemented in this crate -- including a worked packet
set and server flow in [`src/demo/`](src/demo) -- so it can be judged by running
`cargo test -p passage-driver` rather than by reading prose alone.

The first implementation was then reviewed, and three of its decisions were reversed: the `packet!`
macro was replaced by hand-written codecs, the two ways of doing asynchronous work collapsed into one
task model plus a read gate, and the deferred state `Update` became an ordinary operation.
[08-refinements.md](docs/08-refinements.md) records the reasoning; the other documents were updated to
match, and keep the "tried and removed" notes rather than pretending the earlier shape never existed.

## Static, and per connection

Two halves, and the vocabulary follows the split: a `Router` is built once at startup and shared,
a `Connection` is created for each accepted socket and owns everything mutable.

| Static, built once  | One per accepted socket                          |
|---------------------|--------------------------------------------------|
| `Router`            | `Connection`                                     |
| `ConnectionConfig`  | `ConnectionHandle`                               |
| the handlers        | the state `S`, and a `Ctx` per handler call      |

`server::serve` is the bridge, in the shape Axum uses -- with one difference: state here is *per
connection*, so it takes a factory rather than a value and calls it once per socket.

```rust
let listener = TcpListener::bind("0.0.0.0:25565").await?;

serve(listener, router()?, |addr| Session { peer: Some(*addr), ..Session::default() })
    .config(config)
    .with_graceful_shutdown(shutdown)
    .on_finish(log_completion)
    .await;
```

One connection without the accept loop is `Connection::new`, which is what `serve` calls per
socket and what the tests drive over a socket pair.

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
