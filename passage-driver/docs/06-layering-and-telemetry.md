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
| Route matching, adapter selection        | passage   | Deployment concern                                         |

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
called: from a handler that returns `Flow::later`, or from a detached task, instead of from the middle
of a 470-line function. The `Rejected` error mapping to a localized disconnect packet becomes one
helper:

```rust
async fn reject(conn: &ConnHandle<Session>, locale: Option<&str>, key: &str) -> Result<Update<Session>> {
    let reason = localize(locale, key).await?;
    conn.send(&Disconnect { reason })?;
    conn.close()?;
    Ok(Update::none())
}
```

## Telemetry

The current implementation instruments generously; the aim is better structure, not more spans.

### Spans

| Span                 | Parent            | Attributes                                                                 |
|----------------------|-------------------|----------------------------------------------------------------------------|
| `connection`         | session cookie, if present | `client.address`, `server.address`, `mc.protocol.version`, `mc.intent`, `route.name` |
| `packet.read`        | `connection`      | `mc.packet.name`, `mc.packet.id`, `mc.phase`, `mc.packet.size`               |
| `packet.write`       | `connection`      | same                                                                        |
| `handler`            | `packet.read`     | `mc.packet.name`, `handler.async` (whether it returned `Pending`)            |
| `adapter.<kind>`     | `handler`         | adapter type, outcome                                                       |

Two improvements over today:

* **The driver emits the packet spans**, so handlers get instrumentation for free and cannot forget
  it. `Encoded.name` and the router's entry name exist for exactly this -- both are `&'static str`,
  so the label is free and can never contain peer data.
* **`handler.async` makes the pause visible.** A phase that unexpectedly blocks reads shows up as a
  handler span with a long duration, which is the thing you actually want to find.

Trace continuation through the session cookie (already implemented) is worth keeping: it is what
makes a transfer chain across Passage instances one trace.

### Metrics

Keep the existing counters and add the ones the new error model makes possible:

| Metric                             | Type      | Labels                                  |
|------------------------------------|-----------|-----------------------------------------|
| `passage.connections`              | counter   | `outcome` = closed/peer_closed/cancelled/error |
| `passage.connection.errors`        | counter   | `class` = peer/transport/internal, `kind` = `Error::label()` |
| `passage.packets`                  | counter   | `direction`, `phase`, `packet`           |
| `passage.packet.size`              | histogram | `direction`                              |
| `passage.handler.duration`         | histogram | `packet`, `async`                        |
| `passage.protocol.version`         | counter   | `version`                                |
| `passage.connection.duration`      | histogram | `outcome`                                |

`Error::label()` is deliberately a closed set of `&'static str`, so the error metric cannot blow up
cardinality no matter what a peer sends. That is the property today's `err.to_string()` logging does
not have.

### Logging levels, by class

| Class       | Level  | Report to Sentry |
|-------------|--------|------------------|
| `Peer`      | debug  | no               |
| `Transport` | debug  | no               |
| `Internal`  | warn   | yes              |

The point is that this table is decided once, in one function (`log_completion`), rather than at every
call site.
