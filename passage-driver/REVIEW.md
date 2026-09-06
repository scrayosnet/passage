# Critical review of `passage-driver`

A scan of the driver as it stands against the implementation it is meant to replace
(`passage-protocol` + `passage-packets`). Written after the `Dispatcher`/`serve` round.

**Scope.** The driver deliberately implements only a slice of the application flow, and this review
does not count missing packets, adapters, cookies or crypto as findings. What it *does* look for is
the opposite: places where the design, the defaults or the documented contract would get in the way
of finishing the flow, or where behaviour the old implementation had is now harder or impossible to
express. Every claim below is either a code citation or something I reproduced with a throwaway
test; where I reproduced it, the result is quoted.

Findings are ordered by severity. Section [F](#f-investigation) collects the things I think are
wrong or unfinished but do not have a confident answer for.

---

## A. Correctness

### A1. The dispatch table is keyed by *exact* protocol version, so unlisted releases cannot log in

`RouterBuilder::build` inserts one table per version in the iterator it is handed
(`src/router.rs:213-219`), and `Router::table` falls back to the version-independent table for
anything else (`src/router.rs:332-334`). The fallback contains only the packets whose `ids` table is
anchored at `UNKNOWN` — in the demo, `Intention`, `StatusRequest`, `PingRequest`.

`demo::server::SUPPORTED_VERSIONS` lists three numbers: 766, 767, 775
(`src/demo/server.rs:34-35`). Every other real release — 768 (1.21.2), 769 (1.21.4), 770 (1.21.5),
771…774 — therefore gets the fallback table and **cannot get past the handshake**, even though
`Packet::id` resolves perfectly well for it.

Reproduced with a scratch test at protocol 768:

```
assert_eq!(LoginStart::id(ProtocolVersion::new(768)), Some(0x00));  // passes
outcome for protocol 768: Err(Protocol(UnexpectedPacket {
    packet: "Intention", expected: Handshake, phase: Login,
}))
```

Two things make this worse than a missing entry:

1. **It is silent.** Nothing at build time and nothing at connect time says "this version has no
   table". The connection simply behaves as if login did not exist.
2. **The diagnostic actively misleads.** `Table::lookup_elsewhere` (`src/router.rs:121-126`) finds
   *some* packet with id `0x00` in another phase and reports that one — so a missing version table
   surfaces as "the client sent `Intention` in the Login phase", which sends whoever reads the log
   after a protocol-ordering bug that does not exist. The cross-phase hint is a good idea, but it
   needs to lose to "this version has no table at all".

The old implementation had no equivalent failure mode: IDs were `const`s, and the only version
comparison was `client.protocol_version < MIN_PROTOCOL_VERSION`
(`passage-protocol/src/connection.rs:354`), so every version ≥ 766 worked without being enumerated.

The docs describe the list as a range — "anything outside the supported range … gets the
version-independent table" (`src/router.rs:18-21`) — and that word is doing real damage: it is a
*set*, and the gaps between its members are live releases.

**Recommendation.** Derive the breakpoints instead of asking for them. Every `Packet::id` is a step
function over a finite set of thresholds; collect the thresholds of all registered packets, build one
table per interval, and look them up by binary search over a sorted `Vec<(ProtocolVersion,
Arc<Table>)>`. That makes `build()` take no version list at all, makes the fallback unreachable for
any version at or above the lowest threshold, and costs one comparison more per rebind (which only
happens once per connection). The weaker fix — keep the list, but have `build()` reject a list that
does not contain every threshold any registered packet mentions — catches a forgotten *threshold*
but not the gaps, so it does not solve this.

### A2. `at_least` misclassifies snapshot protocol versions

Minecraft snapshots encode their protocol version with the top bit set (`0x40000000 | n`), so every
snapshot compares greater than every release. `ProtocolVersion::at_least`
(`src/version.rs:76-78`) is a plain `>=`, so a 1.20.5 snapshot satisfies
`at_least(versions::V26_2)` and gets a `LoginSuccess` with a trailing `session_id` field it cannot
parse (`src/demo/packets.rs:341-350`, `src/demo/server.rs:168`).

`src/version.rs:137-142` asserts this as intended behaviour:

```rust
assert!(ProtocolVersion::new(i32::MAX).at_least(versions::V1_20_5));
```

which is right for the "hostile garbage must not panic" property it is testing, and wrong as a
statement about snapshots. Old code was unaffected because it never gated a field on a version.

**Recommendation.** Either mask/reject the snapshot bit when decoding the handshake (a snapshot is
not a version this router can reason about), or make the comparison snapshot-aware and document
which of the two it is. Whichever way it goes, `at_least` is currently the *only* sanctioned way to
branch on a version (`src/version.rs:71-75`), so the fix belongs there rather than in each codec.

### A3. A blocked write suspends the whole loop — including both deadlines and the shutdown token

`handle_op` awaits `framed.feed` (`src/conn/connection.rs:265`) and `flush`
(`:335-340`), and it is awaited *outside* the `select!` (`:216`). tokio-util's `Framed` applies
backpressure at an 8 KiB write buffer, so a peer that stops reading parks the connection task inside
`feed`, where nothing else is polled.

Reproduced with a scratch test: a 16-byte socket, a client that never reads, `max_lifetime` and
`max_idle` both 5 s, `shutdown.cancel()` at t=1 s, then 600 simulated seconds:

```
finished after 600s past the 5s deadline: false
```

Three consequences:

- `ConnectionConfig::max_lifetime` is documented as a "Hard cap on the whole connection", and its
  doc argues that it belongs on the connection rather than in a `tokio::time::timeout` precisely
  *because* the connection owns the clock (`src/conn/connection.rs:52-59`). Against a peer that
  simply does not read, it is not a cap at all.
- `max_idle` does not help either: it is a *read* deadline, and the read half is not being polled.
- `Server::drain_timeout` cannot bound the drain. Its escape hatch is `live.cancel()`
  (`src/server.rs:252`), but a connection stuck in `feed` never observes its token, so the
  `tasks.wait()` that follows (`:253`) never returns and `serve` never completes — the process
  cannot shut down.

This is inherited rather than introduced (the old `send_packet` awaited the same way,
`passage-protocol/src/connection.rs:199-208`), but the driver now makes promises about it that the
old code did not, and `serve`'s graceful-shutdown contract is built on top of it. It is also the
cheapest denial-of-service against a Passage instance: open connections, request status, never read.

**Recommendation.** The write must be cancellable. Options, roughly in increasing order of
intrusiveness: race each `feed`/`flush` against `shutdown.cancelled()` and the lifetime sleep inside
`handle_op`; or give the loop a "flush pending" select arm driven by `poll_flush` so writes make
progress inside the select rather than outside it; or add an explicit write deadline to
`ConnectionConfig` and enforce it around every socket await. The middle option is the one that keeps
the "one task owns everything" property intact.

### A4. Returning an error from a handler throws away everything that handler queued

`run` propagates with `?` (`src/conn/connection.rs:216`, `:230`, `:236`) and only reaches
`self.finish().await` on the non-error path (`:240`). So when a handler returns `Err`:

- every `Op` it queued before failing is still in the channel and is never drained;
- the write buffer is not flushed;
- `finish()` — the shutdown-token cancel, the task drop, the socket flush and close — does not run.

That makes the old implementation's error style inexpressible. Every rejection there sent a
localized disconnect and *then* ended the connection: unsupported version
(`passage-protocol/src/connection.rs:367-376`), failed authentication (`:162-174`), missed
keep-alive (`:216-224`), no target found (`:643-655`). In the driver, `ctx.send(Disconnect { .. })`
followed by `Err(Error::peer(..))` silently drops the packet.

The demo already regresses this: `on_tick` returns `Err(Error::peer("keep_alive_timeout", …))`
(`src/demo/server.rs:214-218`) where the old code sent `disconnect_timeout` first. The comment there
explains the *classification* choice, which is right — but the packet is gone.

**Recommendation.** Either drain-and-flush the pending queue on the error path before returning, or
make failure an operation: `Ctx::fail(err)` queueing an `Op::Fail(Error)` so it lands in queue order
behind the sends the handler asked for. The second fits the design's own logic — "everything a
handler does is an operation" (`src/conn/mod.rs:26`) — and failure is currently the one exception.

### A5. Nothing can react to shutdown, a deadline, or a driver-generated error

`Step::Shutdown` and `Step::Expired` break the loop and close (`src/conn/connection.rs:232-233`).
There is no hook, on `Connection`, on `Dispatcher` or on `Server`, that runs at that moment — so
there is no way to send the peer a "server restarting" or "took too long" disconnect. The old
implementation did exactly that when its token fired mid-authentication or mid-target-selection
(`passage-protocol/src/connection.rs:156`, `:550`, both producing `disconnect_timeout`), and carried
a `TODO` to extend it to the read path (`:187`).

The same gap covers driver-generated protocol errors: frame too large, unknown packet, early packet.
Vanilla clients show "Connection Lost — Internal Exception" for a bare socket close and the actual
message for a `Disconnect`, so this is user-visible.

This is the largest missing *capability* for rebuilding the Passage flow on the driver, as opposed to
the largest bug. It also interacts with A4: whatever shape it takes has to be able to write a packet
on a path that currently cannot write anything.

**Recommendation.** A `Dispatcher::on_end(&self, ctx, &EndReason) -> Result<()>` called before
`finish()`, on every exit path including errors, with the writes it queues drained by `finish()`.
That keeps the policy (which message, in which phase, localized how) in the layer that owns policy,
and needs no new concept — it is a tick with a reason attached.

---

## B. Design and API

### B1. `Packet::PHASE` is a single constant, but the protocol reuses packets across phases

`Packet::PHASE` is one `Phase` (`src/packet.rs:101-102`) and the table is keyed by it. The real
packet set does not work that way. From the old crate:

| packet | phases |
|---|---|
| `CookieResponse` | Login `0x04`, Configuration `0x01` — **identical payload** |
| `Disconnect` | Login `0x00` (JSON string), Configuration `0x02` (NBT text component) |
| `KeepAlive` / `Pong` / `PluginMessage` / `KnownPacks` | Configuration and Play |

For `Disconnect` two types are genuinely right, because the *encoding* differs. For
`CookieResponse` — which Passage uses in both phases (`passage-protocol/src/connection.rs:388`,
and `conf_in::CookieResponsePacket` in the configuration wait loop at `:595`) — it is pure
duplication: same fields, same codec, two types whose only difference is a constant. That is the
outcome `docs/02-versioning.md` argues against for versions, arriving through the phase axis
instead.

**Recommendation.** Let the phase come from the registration rather than the type: `.on_in::<P,
_>(Phase::Configuration, handler)` alongside today's `.on`, with `P::PHASE` as the default. Making
`PHASE` a `&'static [Phase]` also works but pushes the ambiguity into `Op::Send`'s check (B2).

### B2. `Op::Send`'s check is a phase-*membership* check, documented as a staleness check

`ConnectionHandle::send` stamps `P::PHASE` — a constant — not a snapshot of the connection's phase
(`src/conn/handle.rs:155-162`), and the connection compares that constant against its current phase
(`src/conn/connection.rs:253-263`). `InternalError::StaleEncoding` describes it as "A handler sees a
*snapshot* of the version and phase, and encodes against it" (`src/error.rs:210`), which is true of
the version and not of the phase.

The behaviour is arguably better than what the docs claim — it catches "send a Login packet while in
Configuration" regardless of ordering. But the name and prose mislead about what is being enforced,
and the check is what makes B1 binding: a packet declared for one phase can never be sent in
another, so the duplicate types are mandatory rather than merely idiomatic.

### B3. `Ctx::spawn` is not a spawn

`Op::Spawn` futures go into a `FuturesUnordered` that is polled by the connection loop
(`src/conn/connection.rs:196`, `:287-293`). So a handler task that blocks, or is CPU-bound, stalls
framing, keep-alives and both deadlines — on a connection whose whole point is that keep-alives keep
flowing while backend selection runs.

The old implementation used a real `tokio::spawn` for target selection
(`passage-protocol/src/connection.rs:548`), so a slow or blocking adapter could not starve the
keep-alive interval. The driver's docs say "Runs a future alongside the connection, while packets
keep being dispatched" (`src/conn/handle.rs:218`), which reads like a spawn and is only true for
well-behaved futures.

This matters concretely for Passage: the discovery chain includes a DNS adapter and a Kubernetes
(Agones) adapter, and a synchronous resolver call or a blocking TLS handshake inside one of them now
takes the keep-alive down with it.

**Recommendation.** Say plainly in the docs that the future is cooperatively scheduled on the
connection task and must not block, and add a detached variant for work that should not be able to
starve the loop (`Ctx::spawn_detached`, backed by `tokio::spawn`, talking back through a cloned
handle — it already has everything it needs). The current tests would not catch this: they only use
`tokio::time::sleep` as the stand-in.

### B4. A panic in a handler is silent

Handler futures are polled on the connection task, so a panic unwinds the task `serve` spawned.
`Server::run` drops that `JoinHandle` (`src/server.rs:232-237`), so nothing logs the panic and
`on_finish` — presented as *the* reporting hook (`:166-178`) — is never called for that connection.
The connection just disappears from the metrics that `on_finish` would have fed.

The old implementation had the same shape, but it did not advertise a completion hook, and the driver
does claim a no-panic discipline (`#![deny(unsafe_code)]`, "the no-panic rules" in
`docs/05-errors-and-hardening.md`). Note also that the driver's own `?`-free paths include
`Op::With` closures, which run arbitrary user code inline on the loop (`:286`).

**Recommendation.** Keep the `JoinHandle` and route `Err(JoinError)` into `on_finish` as an
internal-class error, so a panic is reported exactly like any other internal failure.

### B5. `on_finish` gets no context, so the old metrics cannot be rebuilt on it

Its signature is `Fn(&Result<Completion>)` (`src/server.rs:90`). It cannot see the peer address, the
start time, the phase reached, the negotiated version, or the state `S`. Every one of the old
metrics needs at least one of those:

| old metric | needs |
|---|---|
| `connection_duration` | start instant |
| `open_connections` | a paired begin/end hook |
| `handshake_states{state}` | the handshake intent (lives in `S`) |
| `client_locales{locale}`, `client_view_distances` | `S` |
| `listener_requests{decision}` | an accept-time hook (see B8) |

The connection still owns `S` when it finishes, so handing it over costs nothing.

**Recommendation.** Replace the argument with a struct — `addr`, `started`, `version`, `phase`,
`Result<Completion>`, and `S` by value — and add the accept-side counterpart.

### B6. There is no observability seam anywhere

Beyond B5: the driver emits `trace!`/`debug!` events and creates **no span**. The old implementation
opened a `read_packet` / `write_packet` span per packet with `otel.kind = "server"`
(`passage-protocol/src/connection.rs:186`, `:205`), `#[instrument]`ed every step, recorded
`packet_length`/`packet_id` as span fields, emitted packet-size and packet-byte metrics from the
codec (`passage-packets/src/codec.rs:252-253`, `:289-290`), and injected the OpenTelemetry trace
context into the session cookie so a transfer could be followed across servers (`:694-696`).

None of that has a home in the driver today. There is no per-connection span for fields to attach to,
no packet-level hook, and no way to wrap the outbound path at all — encoding happens inside
`ConnectionHandle::send`, which has no seam. Given that "keep the telemetry and even expand upon it
by having better traces" is one of the four stated requirements in `README.md:25`, this needs API
design, not wiring.

`Dispatcher` being a trait does make an inbound decorator possible
(`Instrumented<RouterDispatcher<S>>`), which is a real benefit of last round's change — but see
[F3](#f3-where-cross-cutting-concerns-should-live) for why that only solves half of it.

### B7. `Listener` is the wrong place for the PROXY protocol

`src/server.rs:36-39` proposes it: "a wrapper that parses a PROXY protocol header and reports the
real client address is an implementation of it". But `Listener::accept` takes `&mut self` and is
awaited inline in the accept loop (`:200`), so parsing a header — which means *reading from the
accepted socket* — serialises every connection behind it, and one peer that connects and sends
nothing stalls all accepts indefinitely.

The old implementation did this inside the per-connection task, after the accept
(`passage-protocol/src/listener.rs:89-115`), which is the only correct place for it. Passage runs
behind HAProxy with `proxy_protocol` configurable, so this is not hypothetical.

**Recommendation.** Drop the suggestion from the docs, and add a per-connection preamble hook that
runs inside the spawned task: `Server::preamble(impl Fn(L::Io, L::Addr) -> Future<Output =
Result<(Io2, Addr)>>)`. Proxy-header parsing, a TLS handshake and a Prometheus-style "rejected before
protocol" counter all fit there.

### B8. `serve` has no admission control

Every accept spawns, unbounded (`src/server.rs:232`). There is no connection cap, no accept
semaphore, and no equivalent of the old per-IP `RateLimiter` (`passage-protocol/src/listener.rs:29`,
`:119-130`) or its `listener_requests{decision}` metric. The docs say to write your own loop for
those (`src/server.rs:34-39`), which is honest — but Passage needs all three, so Passage would not
use `serve`, and `serve` then has no first customer.

**Recommendation.** `Server::max_connections(usize)` (a semaphore permit held by the connection
task) and `Server::on_accept(impl Fn(&L::Addr) -> bool)` are both a handful of lines and keep the
real user on the supported path.

### B9. `serve` is concrete over `Router`, not `Dispatcher`

`src/server.rs:98`, `:224`. The inversion `conn` gained last round does not reach the front door: a
custom dispatcher has to abandon `serve` and write its own accept loop. Defensible for a
batteries-included helper, but worth deciding rather than inheriting.

### B10. Dropping the `Server` future orphans live connections

`live` is owned by `run`'s stack frame (`src/server.rs:191`) and each connection holds
`live.child_token()` (`:228`). Dropping the future — a `select!` on `serve(..)` and a signal, say —
drops `live` without cancelling it, so the spawned connections keep running with nothing left that
can stop them.

**Recommendation.** Hold `live` in a guard that cancels on drop, or document that the future must be
driven to completion.

---

## C. Limits, defaults and hardening

### C1. The default frame limit is smaller than the demo's own field limits, and smaller than the old default

`Limits::max_frame_len` defaults to 8 KiB (`src/wire.rs:57`). The old default was 10,000
(`passage-protocol/src/config.rs:4`). Meanwhile the demo declares `MAX_STATUS_LEN = 32_768` and
`MAX_PROPERTY_LEN = 32_768` (`src/demo/packets.rs:27`, `:30`) — fields that cannot fit in a frame,
so those limits are dead and the effective limit is 8 KiB with a misleading error.

That matters for the one packet a public server always sends: a status response carrying a 64×64
favicon is base64-encoded and routinely exceeds 8 KiB, and `Encoded::of` would refuse it as
`InternalError::OversizedFrame` (`src/codec.rs:86-93`) — our own bug class, for ordinary content.
Signed profile properties in `LoginSuccess` and the auth cookie (protocol cap: 5,120 bytes of
payload) are the same story from the other direction.

**Recommendation.** Raise the default — 10 KiB restores parity, 32 KiB is safer for status — and add
a build-time check that no registered packet's declared field limits exceed `max_frame_len`, since
`build()` already walks every packet. The comment justifying 8 KiB ("the biggest is a status
response it sends itself", `src/wire.rs:56-57`) is exactly the case that breaks it.

### C2. Writes are not checked against per-field limits

`Writer::length` bounds only by `max_frame_len` (`src/wire.rs:402-414`). So reads enforce
`r.string("server_address", 255)` and writes enforce nothing per field — a `Transfer` with a 4 KiB
hostname encodes happily and is rejected by the client. Low severity, but it weakens the module's
own third rule, "Never emit a length that disagrees with its payload" (`:11-13`), which is stated as
being about outbound correctness.

### C3. The operation queue is unbounded

`mpsc::unbounded_channel` (`src/conn/handle.rs:114`). `Op::Send` carries `Bytes`, so a handler task
queueing sends in a loop while the socket is blocked (A3) grows memory with no bound and no
diagnostic. The old `stream.send().await` was self-limiting. A bounded queue with `try_send` and a
`QueueFull` internal error would turn a leak into a classified failure.

### C4. A default `ConnectionConfig` has no timeout of any kind

`tick_interval`, `max_lifetime` and `max_idle` all default to `None`
(`src/conn/connection.rs:73-83`), and `serve` uses `ConnectionConfig::default()` unless told
otherwise (`src/server.rs:126`). So the documented minimal example — `serve(listener, router,
state).await` (`src/server.rs:15-19`) — accepts connections that can idle forever. The old listener
always applied a 120-second cap (`passage-protocol/src/config.rs:10`,
`passage-protocol/src/listener.rs:138`).

Defaults are the security posture here; these should be set to something, or `serve` should refuse a
config with no bound at all.

### C5. `MAX_PACKET_ID = 255` will not cover the Play phase

`src/router.rs:43`. 1.21 clientbound Play IDs already run past `0x80`, and `Phase::Play` is declared
supported ("Passage never reaches this phase, but the driver is not Passage",
`src/packet.rs:47-48`). Latent, and cheap to remove: the table width can be derived from the
registered IDs, leaving the constant as a sanity ceiling rather than a cap.

### C6. `ProtocolVersion::UNKNOWN == 0` overlaps a legal wire value

`src/version.rs:54`. A client may send `0` in the handshake; `on_intention` only refuses negatives
(`src/demo/server.rs:89`). It happens to be harmless — `0` resolves to the fallback table, which is
what a pre-handshake connection gets anyway — but a sentinel inside the domain it guards is worth
replacing with `Option<ProtocolVersion>` or a value no client can send.

### C7. `ids()` requires a descending table and nothing enforces it

`src/packet.rs:135-140` returns the first entry with `version.at_least(since)`. A table written
oldest-first silently resolves to the wrong ID for every version above the second entry — no error,
just a packet the client misparses. `build()` already visits every entry at every version, so
asserting the thresholds are strictly descending there is free.

---

## D. Smaller things

- **`FrameCodec::new` does not infer its defaulted type parameter.** Hit while writing a probe: a
  bare `FrameCodec::new(limits)` is `E0283`, requiring `let c: FrameCodec = …` or a turbofish. Type
  defaults do not participate in inference for associated functions. A `FrameCodec::boxed(limits)`
  constructor, or making the boxed form the only public path, removes the papercut.
- **`Encoded::of` does not check `P::DIRECTION`.** `src/codec.rs:71-99`. The router validates
  direction at registration (`src/router.rs:157-164`) but the send path has no equivalent, so a
  server can encode a serverbound packet. Passing the direction into `ConnectionHandle` would close
  it.
- **Dead public accessors.** `Router::inbound` and `Router::unknown_policy`
  (`src/router.rs:313-323`) have no callers since `RouterDispatcher` reads the fields directly.
- **`wire.rs` is missing primitives the real packet set needs**: NBT (the Configuration-phase
  `Disconnect` and `AddResourcePack` prompt are network-NBT text components,
  `passage-packets/src/writer.rs:105-116`), bool-prefixed optionals (`CookieResponse.payload`,
  `passage-packets/src/login.rs:404-423`), `i8`/`i16`/`i32` big-endian, `f32`/`f64`. Note that
  `Reader::gated` is *version*-gated (`src/wire.rs:268-278`) and is not the bool-prefixed optional —
  the naming invites confusing the two, so the optional wants its own method rather than a reuse.
- **`Phase` has three parallel definitions.** `COUNT`, `ALL` and `index()`
  (`src/packet.rs:52-75`); a new phase needs three edits and only a test ties them together
  (`:160-164`). `#[repr(usize)]` plus `COUNT = ALL.len()` and `index() = self as usize` removes two
  of the three.
- **`InternalError` does not derive `PartialEq`** while `ProtocolError` and `BuildError` do
  (`src/error.rs:39`, `:167`, `:353`), so tests match structurally on one and compare the others.
- **`Error::Closed` is classified `Class::Transport`** (`src/error.rs:305`) but documented as "not a
  failure of the connection … callers usually ignore it" (`:267-270`). A handler that `?`s it ends
  the connection as a transport error. Worth either its own class or a doc note that it is meant to
  be swallowed.
- **`is_peer_error` matches `ConnectionRefused`** (`src/server.rs:283`), which `accept` does not
  produce. Harmless.
- **Nobody sets `TCP_NODELAY`.** Neither implementation does (the old one only in its test harness,
  `passage-protocol/tests/support/mod.rs:109`). With the driver's batched flushes, Nagle can add
  ~40 ms to a status ping — the one latency users see. `serve` is the natural place.
- **`docs/01`–`08` are stale across three rounds** — they still name `Driver`, `DriverConfig` and
  `ConnHandle`, do not know `serve`/`Listener`/`Dispatcher`/`RouterDispatcher` exist, `02` §B2 still
  describes named `Feature` gates, `04` still describes `Op::Send` as encoded at drain time, and the
  layer diagram in `docs/README.md` still says "Driver". Left alone per the standing instruction.
- **`Updates.md` is still in the tree** although it was folded into `docs/08-refinements.md`, and
  `NOTES.md` is staged for deletion.

---

## E. What is clearly better than before

Worth recording, because some of it is load-bearing and should not be traded away while fixing the
above.

- **The reader is bounds-checked; the old one was not.** `passage-packets/src/reader.rs:81-85` does
  `let length = self.read_varint()? as usize; let mut buffer = vec![0; length];` — a negative length
  inside an otherwise valid frame becomes a ~18 exabyte allocation and an abort. The driver rejects
  it before allocating (`src/wire.rs:204-228`), and has the test to prove it
  (`negative_length_is_rejected_before_allocating`). This alone justifies the rewrite.
- **`VarLong` is read at ten bytes, not nine.** The old `read_varlong` loops `0..9`
  (`passage-packets/src/reader.rs:70`), truncating every value above 2⁵⁶ and leaving a byte in the
  stream, which desynchronises everything after it. `src/wire.rs:181-200` gets it right and says why.
- **Non-canonical varints are rejected**, removing a parser-differential surface the old codec had.
- **`bool` matches vanilla.** Old: `bool == 1u8` (`passage-packets/src/reader.rs:88-90`). New: `!= 0`
  (`src/wire.rs:120-122`).
- **Trailing bytes are checked once, centrally** (`src/router.rs:166-174`), where the old code
  checked nowhere — a packet longer than our field list was silently accepted.
- **Errors carry blame.** `Class` (`src/error.rs:22-33`) replaces
  `Ok(()) | Err(Error::ConnectionClosed)` being treated identically at every call site, which is a
  real defect in `passage-protocol/src/listener.rs:164-171`: any new error variant silently lands in
  the wrong bucket.
- **ID collisions are a startup failure.** `BuildError::IdCollision` (`src/error.rs:377-393`)
  catches at boot what the old `match_packet!` chain — a linear `if id == T::ID` cascade
  (`passage-packets/src/codec.rs:78-88`) — resolved silently in registration order.
- **The 470-line `listen()` is gone.** `passage-protocol/src/connection.rs:251-718` is one function
  holding the entire state machine, with a labelled block (`'transfer:`) for control flow and a
  `TODO handle rejected error!` in the middle of it. Each step is now a testable function.

---

## F. Investigation

Things I think are wrong, unfinished, or worth a decision, where I do not have a confident
recommendation.

### F1. Sequential request/response steps have no home

The old flow is a straight line, and each arrow is an *ordering guarantee*: send `CookieRequest` →
await `CookieResponse` → send `EncryptionRequest` → await `EncryptionResponse` → authenticate →
send `LoginSuccess`. `match_packet!` at each step meant anything out of order closed the connection
(`passage-protocol/src/connection.rs:386-394` is the pattern, repeated six times).

In the driver each arrow becomes a separate handler plus a field in `Session`, and **nothing enforces
the order**. A client may send `LoginAcknowledged` before `LoginStart`, or `LoginStart` twice, or a
`CookieResponse` nobody asked for, and the router dispatches all of them. `Phase` is too coarse —
half a dozen ordered steps live inside `Phase::Login` alone. So every handler has to defend itself by
inspecting state, and nothing makes it.

I do not have a clean answer. Three shapes I can see:

- (a) **Accept it**, and make "a handler must validate its own preconditions against `ctx.state`" an
  explicit documented rule with a worked example. Cheapest; relies on discipline for a property the
  old code got structurally.
- (b) **One-shot continuations**: `ctx.once::<CookieResponse>(|ctx, packet| …)` registering a
  handler that supersedes the table for the next matching packet, with anything else being a
  protocol error. Expresses the old guarantee exactly, but reintroduces the sequential coupling the
  registration model was meant to dissolve, and needs a well-defined interaction with `exclusive`.
- (c) **Make the step part of the table**: a per-connection "step" alongside `Phase` that the router
  keys on, so the sequence is data rather than convention. Most faithful, biggest change, and it
  doubles the table dimension.

This is the one open question I would resolve before building the real flow on top, because all three
answers change what handlers look like.

### F2. Compression has no place in the framing layer

`SetCompression` (login `0x03`, `passage-packets/src/login.rs:161-164`) exists as a declared packet
in the old crate and is unused, because Passage never needs it. But the driver bills itself as "a
general-purpose backbone for the Minecraft (Java) protocol" (`src/lib.rs:1`), and a client
implementation talking to a vanilla server *must* handle it — as must anything reaching
`Phase::Play`.

Structurally it looks like encryption: a threshold set mid-stream, applied per frame, switched over
at an exact byte boundary — so `Op::Encrypt`'s ordered-switchover treatment is the obvious model.
What I have not worked out is the nesting: a compressed frame carries an *uncompressed length*
prefix inside the frame, before the packet ID, which means `Frame`/`Encoded` (which are currently
"ID varint + payload", `src/codec.rs:44-64`) grow a third layer. Whether that stays clean, or wants
a separate `CompressedFrameCodec`, I do not know.

### F3. Where cross-cutting concerns should live

Metrics, spans, packet logging, and per-packet limits are all "wrap every dispatch" concerns.
Last round's `Dispatcher` trait makes an inbound decorator possible and pleasant:
`Instrumented<RouterDispatcher<S>>` needs no cooperation from anything.

Two problems keep that from being the answer:

1. A decorator sees `(id, payload)` and not the decoded packet, because decoding happens inside the
   erased handler (`src/router.rs:168-174`). So a "packet name" span field or a
   `packets_total{packet}` counter is reachable from `RouterDispatcher` (which has the name) but not
   from a wrapper around it.
2. There is **no outbound seam at all**. Encoding happens in `ConnectionHandle::send`
   (`src/conn/handle.rs:155-162`), which is a concrete method on a concrete struct. The old codec
   recorded encoded packet sizes and bytes (`passage-packets/src/codec.rs:289-290`); there is
   nowhere to put that now.

A symmetric middleware story would need a seam on both sides, and I do not know what its shape should
be — a `Layer`-style trait over `Dispatcher` covers half, and the write half may want the codec to
become an extension point instead.

### F4. Direction, and whether one router can serve a proxy

`Router::inbound` is a single `Direction` checked at registration (`src/router.rs:157-164`). But
`dispatch.rs:13` names a proxy — "a proxy that forwards everything it does not understand" — as a
motivating use, and a proxy needs a serverbound *and* a clientbound table on one connection, with
two `Phase` cursors that can be out of step. Whether that is two routers, two `Connection`s glued
together, or a direction-keyed table is an open design question that affects whether `Direction`
belongs on `Router` at all.

### F5. `Op::With` allocates per state change

`Op::With(Box<dyn FnOnce(&mut S) + Send>)` (`src/conn/handle.rs:41`) is a box per state touch. For
Passage's handful of updates per connection that is irrelevant. For a driver that means to reach
`Phase::Play`, it is an allocation per state mutation on a hot loop. I do not have a shape that keeps
the "one writer, no locks, no stale reads" property without it — a typed `Op<S>` enum would need the
driver to know the state's shape, and a `&mut S` handed to handlers gives up the whole model.

### F6. `Completion` cannot distinguish "we refused the peer" from "it went fine"

A connection that ended because we sent a `Disconnect` reports `Completion::Closed`
(`src/conn/connection.rs:30-31`) — identical to a status ping completing normally, identical to a
transfer succeeding. The old code could not tell them apart either, so this is not a regression, but
"how many clients did we refuse, and why" is the first thing an operator asks and the metric is
currently unbuildable. Whether that is a `Completion` variant, a payload on `Closed`, or something
the handler records in `S` before closing depends on F1 and B5, so I have left it open.
