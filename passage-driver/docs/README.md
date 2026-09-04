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
| [04-runtime.md](04-runtime.md)                                 | How a connection is driven and ordered               |
| [05-errors-and-hardening.md](05-errors-and-hardening.md)       | Error taxonomy, limits, and the no-panic rules       |
| [06-layering-and-telemetry.md](06-layering-and-telemetry.md)   | Crate layering, adapters, tracing and metrics        |
| [07-reference-implementation.md](07-reference-implementation.md)| The working code, and what it proves                 |
| [08-refinements.md](08-refinements.md)                         | The review of the first implementation, and what changed |

## The five decisions at a glance

| # | Decision            | Options                                                                                            | Recommended                                       |
|---|---------------------|----------------------------------------------------------------------------------------------------|---------------------------------------------------|
| 1 | Packet identity     | `const ID` per version-specific type · `fn id(version)` + hand-written dispatch · declarative ID table · codegen from `minecraft-data` | **Declarative ID table** (`ids()`), hand-written codecs |
| 2 | Version differences | one type per version · `Option<T>` + named feature gates · per-field DSL                             | **`Option<T>` + feature gates**, in a hand-written codec |
| 3 | Dispatch            | one big hooks trait · visitor · typed registration in a router · typestate phases                   | **Typed registration** (`RouterBuilder::on::<P>`), tables built at startup |
| 4 | Runtime             | sequential `&mut self` · one task + ordered op queue · split read/write tasks                        | **One task + ordered op queue**; every handler effect is an `Op` |
| 5 | Errors              | one flat enum · blame-classified enum + explicit completion                                          | **Blame-classified**, `Completion` is not an error, build errors are separate |

Rows 1, 2 and 4 were revised after the first implementation was reviewed. Both readings are kept
deliberately: the documents record what was recommended, tried, and then reversed, along with the
reasoning. [08-refinements.md](08-refinements.md) is the review; the "tried and removed" notes in
[02-versioning.md](02-versioning.md) and the option comparisons in [04-runtime.md](04-runtime.md) are
where the reversals are argued.

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
|   Router      typed registration -> per-version (phase, id)   |
|               tables, built once at startup                   |
|   Driver      one task: ops, handler tasks, deadlines, ticks,  |
|               frames -- and the only owner of state/phase/    |
|               version/socket                                  |
|   FrameCodec  length prefix, packet id, optional encryption   |
|   wire        bounds-checked primitives, per-field limits     |
|   Packet      hand-written codecs + declarative ids()          |
|   error       Peer / Transport / Internal + Completion,        |
|               BuildError kept separate                        |
+--------------------------------------------------------------+
```

Data flow for one packet:

```text
bytes -> FrameCodec -> Frame{id, payload}
      -> gate: exclusive task in flight? -> EarlyPacket
      -> table(version).lookup(phase, id)     <- table shared, not built per connection
      -> P::decode(Reader, version)           <- version-gated fields and per-field limits here
      -> reader.finish(P::NAME)               <- once, in dispatch; no codec can forget it
      -> handler(Ctx{&Session, &ConnHandle}, P) -> Result<()>
      -> Op::Send / With / SetPhase / SetVersion / Spawn / Encrypt / Close
                                              <- drained in order, before the next frame
```

Everything on that last line is queued, never applied in place. That is what makes "record the
profile, then announce it" and "send this, then switch to encryption" mean what they say.

## Non-goals

* Not a proxy. The driver never needs to forward play-phase traffic, so packet coverage stays small
  and the codec can keep hard limits that a proxy could not.
* Not a full protocol database. Version tables are filled in as packets are needed, and validated
  against the supported range at startup rather than being complete for its own sake.

## Reading order

If you only read two: [02-versioning.md](02-versioning.md) for the packet layer and
[04-runtime.md](04-runtime.md) for the driver. Those two contain the decisions that are expensive to
change later.

If you read the earlier revision of these documents,
[08-refinements.md](08-refinements.md) is the shortest path to what changed and why.
