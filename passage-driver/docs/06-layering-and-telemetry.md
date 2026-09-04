# 6. Layering, adapters and telemetry

## Crate layering

The requirement is three tiers: a backbone, a server and client on top of it, and the Passage router
on top of those.

```text
passage                       binary: config, wiring, listener, shutdown, observability setup
  |
  +-- passage-server          the Minecraft server flow as handlers over a Session
  |     +-- passage-adapters  status / auth / discovery / localization traits (unchanged)
  +-- passage-client          the client flow (testing, health checks, future federation)
  |
  +-- passage-driver          framing, dispatch, ticks, shutdown, errors, wire primitives
  +-- passage-protocol-types  packet declarations, version table, features
```

Two deliberate changes from today's layout:

* **`passage-packets` splits.** The wire primitives and the `Packet` trait belong in the driver
  (they are what the driver's codec is written against); the packet *declarations* belong in their
  own crate so a third party can add packets without forking the driver. This crate keeps both
  together only to stay self-contained as a proposal.
* **`passage-protocol` disappears as a concept.** Its content divides cleanly into "driver"
  (listener, codec, connection loop) and "server" (the flow, cookies, crypto policy).

### Which layer owns what

| Concern                                | Layer     | Why                                                       |
|----------------------------------------|-----------|-----------------------------------------------------------|
| Length prefix, packet ID, encryption    | driver    | Independent of who is speaking                             |
| Version table, packet fields            | types     | Data, shared by server and client                          |
| Limits, deadlines, unknown-packet policy | driver (config) | Enforced in one place, configured per route          |
| Phase transitions, keep-alive policy     | server    | Protocol semantics                                         |
| Cookies, encryption handshake, auth      | server    | Passage policy, not protocol mechanics                     |
| Log levels and metric emission           | server / passage | *What* to record is policy; the driver only supplies enough to decide it |
| Route matching, adapter selection        | passage   | Deployment concern                                         |

The `log_completion` helper is the smallest example of the second-to-last row: it started out in
`driver.rs` and belongs above it, because a library that picks its own log levels has taken a
decision away from its caller. What the driver owes the layer above is `Completion` for the happy
paths and `Class` + `label()` for everything else. It currently lives in `demo::server`, and moves to
`passage` with the rest of the observability wiring.

### `Router` per route, or per process?

Passage matches a hostname to a `Route` carrying its own adapters. Two options:

| Option                                                                 | Pros                                                   | Cons                                                                 |
|------------------------------------------------------------------------|--------------------------------------------------------|----------------------------------------------------------------------|
| One `Router` per process; the route lands in the `Session` after the handshake | One table, built and validated once; adapters resolved by the handler | Handlers must handle "route not selected yet"                        |
| One `Router` per route                                                  | Handlers can close over their adapters                  | N tables; the handshake still has to run on a router that has no route |

**Recommendation: one router per process.** The handshake handler resolves the route and stores it in
the session; every later handler reads `ctx.state.route`. This also keeps the adapters out of the
type parameters -- the `Routes<Stat, Disc, Auth, Loca>` generic chain that makes today's signatures
unwieldy collapses into one field on a plain struct.

### Adapters stay as they are

`StatusAdapter`, `AuthenticationAdapter`, `DiscoveryAdapter`, `DiscoveryActionAdapter` and
`LocalizationAdapter` are a good seam and none of this changes them. What changes is *where* they are
called: from a task the handler hands to `ctx.exclusive` or `ctx.spawn`, instead of from the middle of
a 470-line function. The `Rejected` error mapping to a localized disconnect packet becomes one
helper:

```rust
async fn reject(conn: &ConnHandle<Session>, locale: Option<&str>, key: &str) -> Result<()> {
    let reason = localize(locale, key).await?;
    conn.send(Disconnect { reason })?;
    conn.close()
}
```

An adapter that rejects a player is a `Class::Peer` failure, not an internal one, and the adapter
layer is where that call is made:

```rust
Err(Rejected::NotWhitelisted) => Err(Error::peer("not_whitelisted", err)),
Err(Rejected::Unreachable(e)) => Err(Error::internal("discovery_unreachable", e)),
```

## Telemetry

The current implementation instruments generously; the aim is better structure, not more spans.

### Spans

| Span                 | Parent            | Attributes                                                                 |
|----------------------|-------------------|----------------------------------------------------------------------------|
| `connection`         | session cookie, if present | `client.address`, `server.address`, `mc.protocol.version`, `mc.intent`, `route.name` |
| `packet.read`        | `connection`      | `mc.packet.name`, `mc.packet.id`, `mc.phase`, `mc.packet.size`               |
| `packet.write`       | `connection`      | same                                                                        |
| `handler`            | `packet.read`     | `mc.packet.name`                                                            |
| `handler.task`       | `handler`         | `mc.packet.name`, `exclusive` (whether the peer was gated)                   |
| `adapter.<kind>`     | `handler.task`    | adapter type, outcome                                                       |

Two improvements over today:

* **The driver emits the packet spans**, so handlers get instrumentation for free and cannot forget
  it. `Encoded.name` and the router's entry name exist for exactly this -- both are `&'static str`,
  so the label is free and can never contain peer data.
* **`exclusive` makes the gated window visible.** A handler is now always synchronous, so the
  interesting duration is not the handler's but the *task's*: a `handler.task` span with
  `exclusive = true` and a long duration is a window in which the peer was forbidden to speak, which
  is exactly the thing you want to find. The driver knows both facts (it holds the flag and the
  counter), so the span belongs to it rather than to the handler that started the task.

Trace continuation through the session cookie (already implemented) is worth keeping: it is what
makes a transfer chain across Passage instances one trace.

### Metrics

Keep the existing counters and add the ones the new error model makes possible:

| Metric                             | Type      | Labels                                  |
|------------------------------------|-----------|-----------------------------------------|
| `passage.connections`              | counter   | `outcome` = closed/peer_closed/cancelled/timed_out/error |
| `passage.connection.errors`        | counter   | `class` = peer/transport/internal, `kind` = `Error::label()` |
| `passage.packets`                  | counter   | `direction`, `phase`, `packet`           |
| `passage.packet.size`              | histogram | `direction`                              |
| `passage.task.duration`            | histogram | `packet`, `exclusive`                    |
| `passage.protocol.version`         | counter   | `version`                                |
| `passage.connection.duration`      | histogram | `outcome`                                |

`Error::label()` is deliberately a closed set of `&'static str`, so the error metric cannot blow up
cardinality no matter what a peer sends. That is the property today's `err.to_string()` logging does
not have. Note that `Error::Handler` extends the set without breaking it: a handler supplies its own
`&'static str` label, so the layer above can distinguish `not_whitelisted` from
`keep_alive_timeout` without the driver knowing either name.

Two labels worth watching once this is deployed, because they are new and they mean something
specific:

* `early_packet` -- a peer spoke while it was supposed to be waiting. A steady trickle is scanners
  and odd clients; a step change after a release is the strictness trade in
  [04-runtime.md](04-runtime.md#the-strictness-trade) going wrong, and the signal to reconsider it.
* `unexpected_packet` -- a packet from the wrong phase, by name. Distinguishing this from
  `unknown_packet` is the difference between "a client is confused" and "we are missing a definition".

### Logging levels, by class

| Class       | Level  | Report to Sentry |
|-------------|--------|------------------|
| `Peer`      | debug  | no               |
| `Transport` | debug  | no               |
| `Internal`  | warn   | yes              |

The point is that this table is decided once, in one function (`log_completion`), rather than at every
call site -- and that a handler can put its own failures in the right row by classifying them, rather
than having every rejection land in `Internal` and page someone.
