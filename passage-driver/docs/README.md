# Passage Driver -- design proposals

These documents propose a structure for the next Passage iteration: a general-purpose Minecraft
protocol library (the "driver"), a server and client on top of it, and the Passage router on top of
that.

They are written to be argued with. Each one lays out the options for a single decision, what each
option costs, and a recommendation. The recommendations are consistent with each other and are
implemented in this crate -- see [07-reference-implementation.md](07-reference-implementation.md).

| Document                                                       | Decision                                             |
|----------------------------------------------------------------|------------------------------------------------------|
| [01-problem-analysis.md](01-problem-analysis.md)               | What the current implementation gets wrong, and why  |
| [02-versioning.md](02-versioning.md)                           | How packets carry protocol-version differences       |
| [03-dispatch.md](03-dispatch.md)                               | How packets reach protocol logic                     |
| [04-runtime.md](04-runtime.md)                                 | How a connection is driven, ordered and backpressured |
| [05-errors-and-hardening.md](05-errors-and-hardening.md)       | Error taxonomy, limits, and the no-panic rules       |
| [06-layering-and-telemetry.md](06-layering-and-telemetry.md)   | Crate layering, adapters, tracing and metrics        |
| [07-reference-implementation.md](07-reference-implementation.md)| The working code, and what it proves                 |

## The five decisions at a glance

| # | Decision            | Options                                                                                            | Recommended                                       |
|---|---------------------|----------------------------------------------------------------------------------------------------|---------------------------------------------------|
| 1 | Packet identity     | `const ID` per version-specific type · `fn id(version)` + hand-written dispatch · declarative table · codegen from `minecraft-data` | **Declarative table** (`packet!`), codegen later  |
| 2 | Version differences | one type per version · `Option<T>` + named feature gates · per-field DSL                             | **`Option<T>` + feature gates**, generated codec   |
| 3 | Dispatch            | one big hooks trait · visitor · typed registration in a router · typestate phases                   | **Typed registration** (`Router::on::<P>`)         |
| 4 | Runtime             | sequential `&mut self` · one task + ordered op queue · split read/write tasks                        | **One task + ordered op queue**, `Flow` for async  |
| 5 | Errors              | one flat enum · blame-classified enum + explicit completion                                          | **Blame-classified**, `Completion` is not an error |

## The resulting architecture

```text
+--------------------------------------------------------------+
| passage            config, adapter wiring, listener, metrics |
+--------------------------------------------------------------+
| passage-server / passage-client                              |
|   the protocol flow as handler functions over a Session      |
|   (status, login, encryption, cookies, transfer)             |
+--------------------------------------------------------------+
| passage-driver                                               |
|   Router      typed registration -> (phase, id) table        |
|   Driver      one task: ops, pending handler, tick, frames    |
|   FrameCodec  length prefix, packet id, optional encryption   |
|   wire        bounds-checked primitives + Limits              |
|   packet!     declarative packets with per-version ids/fields |
|   error       Peer / Transport / Internal + Completion        |
+--------------------------------------------------------------+
```

Data flow for one packet:

```text
bytes -> FrameCodec -> Frame{id, payload}
      -> Bound(version).lookup(phase, id)
      -> P::decode(Reader, version)          <- version-gated fields resolved here
      -> handler(Ctx{&mut Session, &ConnHandle}, P) -> Flow
           Ready  -> apply state update, read next frame
           Pending-> stop reading; poll handler; apply update; resume
      -> Op::Send / Op::Encrypt / Op::Close   <- drained in order, before the next frame
```

## Non-goals

* Not a proxy. The driver never needs to forward play-phase traffic, so packet coverage stays small
  and the codec can keep hard limits that a proxy could not.
* Not a full protocol database. Version tables are filled in as packets are needed, and validated
  against the supported range at startup rather than being complete for its own sake.

## Reading order

If you only read two: [02-versioning.md](02-versioning.md) for the packet layer and
[04-runtime.md](04-runtime.md) for the driver. Those two contain the decisions that are expensive to
change later.
