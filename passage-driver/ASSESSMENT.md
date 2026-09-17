# Architectural assessment of `passage-driver`

An outside read of the proposal, against [`REVIEW.md`](REVIEW.md), the crate documentation, and the
implementation it is meant to replace (`passage-protocol`, `passage-packets`). The question is not
"is this code good" -- it is well made, and `cargo test -p passage-driver --all-features` passes 87
tests plus doctests -- but **is this the right shape, and what did the alternatives cost?**

Everything below is stated critically on purpose. The parts that are right are stated briefly; the
parts that are contestable get the space.

---

## Verdict in one paragraph

The four bottom layers -- `wire`, `version`, `packet`, `codec` -- are the strongest part of the
proposal and are a clear improvement over what exists today. They solve the problem that actually
motivated the rewrite (multi-version support without duplicating packet types) and they solve it
with less machinery than the alternatives. The top layers -- `router`, `conn`, `server` -- are a
competent framework that answers a question Passage may not be asking: the protocol flow it routes
is short, fixed and sequential, and the driver converts it from control flow into a set of handlers
plus a nullable state struct. That trade buys real things (keep-alives concurrent with backend
selection, per-packet testing, no 470-line function) and costs real things (sequencing becomes a
runtime check, every state write is a boxed closure, and the operation queue is unbounded). Two
findings below are, in my reading, defects rather than trade-offs: **in-flight work is dropped
without the ending handler being able to see it** (§4.1), and **the driver has no metrics channel
at all** (§4.2), which contradicts a stated requirement. Neither is fatal; both are cheaper to fix
now than after `passage-server` is written on top.

---

## 1. The decisions, and what else was on the table

### 1.1 Protocol flow: linear `async fn` vs. registered handlers

| Option | Shape | Pro | Con |
|---|---|---|---|
| **A. Linear state machine** (today's `Connection::listen`) | One `async fn`, `await` per step | Sequence is the control flow -- you cannot acknowledge a login you never started; the compiler and the reader both see the order | Concurrency needs hand-rolled `select!`; one 470-line function; untestable in pieces; adding a packet edits the middle of it |
| **B. Registered handlers** (chosen) | `.on::<P>(f)`, table dispatch | Adding a packet is one line and breaks nothing; each step is a unit; concurrency is `spawn`/`exclusive` rather than a `select!` arm | **Sequence becomes a runtime check against nullable state.** `on_login_start` tests `profile.is_some()`, `on_login_acknowledged` tests `profile.is_none()` (`src/demo/server.rs:148,190`) -- both were free in option A |
| **C. Trait with a method per packet** | `impl ServerHandler` | Exhaustiveness: you cannot forget a packet | Adding a packet is a breaking change to every implementor -- the thing `router.rs` explicitly rejects, correctly |
| **D. Typestate phases** | `Connection<Login>`, handlers registered per phase-type | Sequence violations become compile errors | Cannot express "any packet in this phase"; a version-dependent phase change becomes a type change; large jump in complexity |

The crate documents C's flaw well and never mentions D. D is the option that would have recovered
what B gave up, and it deserved a paragraph in `REVIEW.md` before B was settled -- not because it is
better (it probably is not, at this size) but because the argument for B currently reads as "not C"
rather than "not D".

**My reading:** B is defensible, but the honest summary is *"we traded structural sequencing for
extensibility"*, and the crate docs phrase it as a neutral fact of life (`src/conn/mod.rs:109-115`,
"Sequence is the handler's business") rather than as the cost it is. For a protocol with six
serverbound packets in a fixed order, that is the single most debatable call in the proposal.

**An option nobody wrote down:** take `wire` + `version` + `packet` + `codec` and keep the linear
flow. That is roughly 1,600 lines of the driver, solves the versioning requirement completely, and
leaves `listen()` readable in protocol order. It does not solve "keep-alives while selecting a
backend" -- which is the one place today's code genuinely tangles -- so I do not recommend it. But
it is the cheapest thing that meets the stated requirements, and it should be on the record as the
baseline the framework has to beat.

### 1.2 Versioning: ID tables as data

| Option | Pro | Con |
|---|---|---|
| **A. One packet type per version** (today, `passage-packets`) | Nothing to resolve at runtime | Combinatorial duplication; the thing the requirements explicitly forbid |
| **B. `IDS: &[(Version, i32)]` as data + `at_least` gates** (chosen) | Both directions derive from one declaration; `Router` reads *thresholds* out of the data and builds a table per actual change, so there is no supported-version list to forget; `grep at_least` is a complete inventory of version-gated fields | Two places still have to agree per gated field (the codec's `at_least` and the handler's `.then(...)`, `src/demo/server.rs:179`) -- it fails closed via `MissingField`, but it is duplication |
| **C. `Feature` enum indirection** | Names the *what* | Rejected in `src/version.rs:25-44`, with the right reasons -- it had already rotted at two variants |
| **D. Codegen from protocol data** (wiki/Burger dumps) | Scales to the full vanilla packet set | Generated decoders cannot express per-field limits or domain types, which `src/packet.rs:12-27` argues persuasively against |

**This is the best decision in the crate.** "No list of supported versions exists to be forgotten"
is a genuine structural property, not a slogan, and `ProtocolVersion::placed()`
(`src/version.rs:125`) unifying the inbound and outbound answer for unplaceable versions is exactly
the kind of bug class you want closed by construction rather than by a test. The snapshot-bit
handling is correct and non-obvious.

Minor: `Packet::DIRECTION` (`src/packet.rs:104`) and `Direction::flip` (`src/packet.rs:87`) are
declared, implemented by every demo packet, and read by nothing. Either the builder should reject a
router mixing directions, or the const should go.

### 1.3 Handler effects: operation queue vs. `&mut S`

Chosen: handlers get `&S` and queue `Op`s; the connection drains them with priority.

| Option | Pro | Con |
|---|---|---|
| **A. Operation queue** (chosen) | One writer, no locks, no stale reads, ordered side effects, all-or-nothing batches; background tasks and handlers use the *same* vocabulary | A `Box<dyn FnOnce>` allocation per state write (`src/conn/handle.rs:262`); read-modify-write inside a handler is impossible without a closure (`src/demo/server.rs:218-222` is the tell); a handler cannot observe its own effects |
| **B. `&mut S` for handlers, queue for handles** | No allocation, natural `if x { x = y }` | Borrow-wise it works (`state`, `handle` and `dispatcher` are disjoint fields), but it reintroduces the hazard the design exists to remove: a handler that mutates and *then* fails has half-applied |
| **C. Handler returns a list of effects** (`Vec<Op>`) | Pure, trivially testable | Loses `spawn` and anything a background task needs; two vocabularies instead of one |
| **D. `Arc<Mutex<S>>`** | Familiar | Everything the module docs say it is |

A is the right call and the reasoning in `src/conn/mod.rs:26-46` is sound. What is undersold is the
cost: because a background task *must* use the queue (it holds no `&mut S`), the queue has to exist
either way -- the question was only whether handlers *also* go through it, and the answer "yes, for
uniformity" costs an allocation on every `ctx.update`. On a status ping that is two allocations; on
a login it is a handful. It does not matter at Passage's volumes. It would matter if the driver ever
reached `Phase::Play`, which the crate repeatedly says it wants to support.

### 1.4 Waiting: the read gate

`exclusive` (peer must stay quiet) vs. `spawn` (overlap) vs. `detach` (own task) is a genuinely good
three-way split, and making `EarlyPacket` an error rather than buffering the frame is the correct
strictness for this protocol.

One risk worth naming: pipelining. Many ping tools write handshake + status-request + ping-request in
a single segment. The priority order saves this case -- ops drain before the next frame is looked at,
so the phase change from `on_intention` lands first (verified by reading the `biased` select at
`src/conn/connection.rs:403-427`) -- but it only saves it because no status handler is `exclusive`.
The moment Passage puts an adapter call behind `exclusive` in a phase where a client legitimately
pipelines, that client becomes a protocol error. That constraint is real and is not documented.

### 1.5 Everything before the protocol: `Layer`

Chosen: one `Layer` trait, composed with `Stack<A, B>`, set with `Server::layer`.

| Option | Pro | Con |
|---|---|---|
| **A. `Listener::prepare` + `Pending`** (built, then removed) | Listener owns its own preamble | A TLS handshake is not a property of a listener; needed an associated type to work around `&self` |
| **B. `Layer` trait, static composition** (chosen) | One shape for PROXY, TLS and rate limiting; a layer may change the socket *type*; the accept loop names none of them; refusal keeps its reason where the reason is | **Not dyn-compatible** (RPITIT at `src/server.rs:199`), so the layer stack is fixed at compile time |
| **C. `tower::Layer`/`Service`** | Ecosystem, ready-made middleware | `Service` is request/response; a socket preamble is neither, and it would pull tower into a crate that currently depends on almost nothing |
| **D. Boxed async callbacks** | Runtime-composable | Cannot express a socket-type change, which is the whole point of the TLS case |

B is the better abstraction, and the argument for it in §F1 of the review is the best piece of
reasoning in that document. **But its practical cost is not written down anywhere, and it lands
directly on Passage.** Today's config has `proxy_protocol: Option<ProxyProtocol>`
(`passage-protocol/src/config.rs:24`) and an optional rate limiter -- both runtime switches. With
`Server::layer` returning `Server<L, F, M, Stack<A, A2>>`, this does not compile:

```rust
if config.proxy_protocol.is_some() {
    server = server.layer(ProxyProtocol::new(trusted));   // different type
}
```

The workarounds are a layer that is internally a no-op when disabled, or a `Box`-free enum layer, or
building the whole server inside a match. All are fine; none are obvious; none are mentioned.
Whoever writes `passage-server` will hit this on day one. Either document the "a disabled layer is a
layer that admits everything" pattern with an example, or ship an `Either<A, B>` layer.

The same RPITIT choice makes `Listener` non-dyn-compatible, which is fine (one listener per server)
but worth knowing before someone tries `Box<dyn Listener>` for a config-selected transport.

### 1.6 Reporting: no hook at all

Chosen: the driver opens a `connection` span and emits one event at the end; there is no
`on_finish`. See §4.2 -- this is where I disagree with the proposal.

### 1.7 Construction: typestate builder

`Server<L, F, M, A>` starting at `Server<(), (), ()>` is elegant and delivers "you cannot forget
one". The costs are real and mostly acknowledged: the failure mode for a forgotten setter is "method
`run` not found", which is among the worse diagnostics Rust produces; `listener` must come first for
inference; and `IntoFuture` has to box (`src/server.rs:644`, correctly recorded as a language
limitation, not a choice). A plain `build() -> Result<Server>` would trade a clear runtime error for
a compile-time one. I think the chosen trade is right for a crate with exactly three required
inputs, and wrong to generalise from.

### 1.8 Error model

The `Error` / `Reason` split (`src/error.rs:18-31`) is the right decomposition and the reasoning is
airtight: `Reader::var_int` has no business returning "the shutdown token was cancelled". `Class`
carrying blame, and `Error::peer`/`Error::internal` letting a handler classify its own failures, is
a direct fix for a real defect in today's code, where every rejection was logged at `warn` and
reported. Making normal completion `Ok` rather than `Err(ConnectionClosed)` fixes the other one.

No criticism here. This section is simply correct.

---

## 2. What is load-bearing and should not be traded away

* **Tables built at startup.** An ID collision is a boot failure, not a runtime error on the first
  client that sends the packet (`src/router.rs:268-307`).
* **`ProtocolVersion::placed()`** as the single answer for unplaceable versions, used by both
  directions. This closes a real class of desynchronisation.
* **`wire`'s three rules** (`src/wire.rs:5-13`) and the per-field limits. The negative-length and
  huge-length tests are the ones that matter, and they exist.
* **The `Dispatcher` seam.** `conn` names no router; `tests/dispatch.rs` exists to fail if that
  creeps back. This is what makes the connection loop testable at all.
* **Permit taken before accept.** Backpressure rather than accept-to-refuse.
* **`StaleEncoding`.** A queued packet encoded for a version the connection has left is refused
  rather than written. Cheap, and catches a bug that is undiagnosable from either end.

---

## 3. Requirements check

| Requirement (from `README.md`) | Status |
|---|---|
| Packets not hardcoding ID and codec; react to version | **Met**, and well |
| Backward compatibility without duplicating packet types | **Met** |
| Backbone does only framing, dispatch, ticks, shutdown, errors | **Met** |
| Follows Ktor/Axum/Tonic shape | **Met** -- registration over trait-implementation, `State` analogue as a per-connection factory |
| Server implementation on top | **Demo only.** `demo::server` is a sketch: no encryption handshake (the `Cipher` seam exists, nothing uses it), no cookies, no resource packs, no adapters |
| Client implementation on top | **Not started.** The router is direction-agnostic, so nothing blocks it |
| Passage router + adapters on top | **Not started** |
| Keep telemetry and expand on it | **Regression.** See §4.2 |

Additionally absent, and worth knowing before this is called a "general-purpose backbone for the
Minecraft protocol":

* **Compression.** There is no `SetCompression` support and no seam for one. It is implementable in
  `FrameCodec` the same way `Cipher` is, but it is not the same shape: the inbound
  `FrameTooLarge` check (`src/codec.rs:171`) bounds the *compressed* length, and with compression
  enabled the number that has to be bounded is the *decompressed* one, or the driver has a zip-bomb
  surface. `Limits` would need a second field and `Encoded::of`'s outbound check
  (`src/codec.rs:86`) would need to move after compression. Passage itself never enables
  compression, so this is not blocking -- but every claim in the docs about reaching `Phase::Play`
  is false until it exists.
* **NBT.** Acknowledged in the demo. It means the configuration-phase disconnect cannot be sent,
  which is why `on_error` only speaks in the login phase (`src/demo/server.rs:283`). Passage needs
  it.
* **Legacy ping (`0xFE`).** Not handled, and not length-prefixed, so it arrives as a malformed
  frame. Worth a decision rather than an omission.
* **`max_frame_len` defaults to 32 KiB in both directions.** Fine for handshake/status/login; not
  fine for `Phase::Play`.

---

## 4. Findings I consider defects

### 4.1 In-flight work is dropped where the docs say it is released

The README and `src/conn/mod.rs:98-102` both make this promise:

> A client that disappears while its backend is being selected has left a selection running, and
> releasing it is the same job as releasing it after a timeout. `on_error` is the one place that job
> can live.

The implementation cannot keep it. The order in `Connection::run` is:

1. `drive()` returns `Err(ending)`
2. `settle()` -- drains what is queued
3. `dispatcher.on_error(ctx, &ending)` -- **tasks are still in flight and have not been polled**
4. `settle()` again
5. `finish()` → `self.tasks.clear()` (`src/conn/connection.rs:602`) -- the futures are dropped

So a `spawn`ed task that has acquired something external but has not yet recorded it in `S` via
`Op::With` is cancelled with nothing having observed it, and `on_error` -- which ran *before* the
drop -- had no way to see it either. For the demo this is theoretical. For Passage it is not:
`select_backend()` stands in for the discovery chain, and an Agones allocation is exactly a resource
that is acquired remotely before anything local knows about it.

The mechanism the crate actually provides is cancellation-on-drop, which means **the release has to
live in the `Drop` of whatever the task holds, not in `on_error`.** That is a perfectly good answer
-- it is the standard Rust answer -- but it is the opposite of what the docs say, and it changes how
the discovery adapter must be written.

*Fix, cheapest first:* correct the documentation to say that a `spawn`ed future is cancelled at the
drop point and must clean up in `Drop`; or give `on_error` the in-flight task count so a handler can
at least tell; or drain tasks with a short deadline before `on_error` instead of clearing them
after.

### 4.2 There is no metrics channel, and "keep the telemetry" was a requirement

`Server` computes exactly what a metric wants -- `elapsed`, `Ending::label()`, version, phase -- and
then prints it (`src/server.rs:713`, `log_ending`). `Outcome` is dropped
(`src/conn/connection.rs:118`). There is no hook.

Today's implementation has `metrics::requests::accept/reject`, `metrics::connection_duration`,
`metrics::open_connections` (`passage-protocol/src/metrics.rs`), all OpenTelemetry. To reproduce
those on the driver you must write a `tracing` layer that matches on the message string
`"connection ended"` and reads the `ending` field. That is string-matching a log line to recover a
number the code already had in a typed form, and it breaks silently when someone rewords the
message. The crate's own tests already do this -- `tests/common` grows a ~40-line log recorder,
which `REVIEW.md` §F2 calls "arguably the better test". It is a worse test: it asserts on prose.

The review's argument for removing `on_finish` (§C1, §F2) is that a hook told about every fate forces
each layer to flatten what it knows into a vocabulary `driver` invents. **That argument is
correct and does not apply to what was removed.** A hook carrying *only what the driver knows* --
`(Duration, &Result<(), Ending>, Option<(ProtocolVersion, Phase)>)`, which is precisely
`log_ending`'s own signature -- imposes no vocabulary on any layer, because no layer reports through
it. The driver is not "the place furthest from where it happened" for the driver's own facts; it is
the only place they exist.

The principle "each layer keeps its own books" was derived from a real problem (a `Refusal` enum
summarising a TLS error into nothing) and then applied one level too far, where it removed a data
channel rather than a coupling. The result is a driver that records its own facts in the one format
that cannot be aggregated.

*Fix:* re-add a single hook with `log_ending`'s exact signature, defaulted to the current logging
behaviour, and keep the log line. Nothing about the layer argument is given up; one string-matching
tracing layer is.

### 4.3 The operation queue is unbounded

`mpsc::unbounded_channel()` (`src/conn/handle.rs:181`). Handlers cannot overrun it -- they run on the
connection's own task, so they cannot outpace the drain. `detach`ed tasks and any holder of a
`ConnectionHandle` can, and `Op::Send` carries fully encoded `Bytes`.

Two consequences, and the second is the more interesting one:

* Memory growth per connection is bounded only by `max_lifetime`. The socket write path *is*
  bounded (`Framed` flushes at its backpressure boundary), so this is specifically the queue.
* The select is `biased` with ops first (`src/conn/connection.rs:406`). While ops keep arriving,
  arms 2-6 -- finished tasks, shutdown, deadline, tick, **and the next frame** -- never run. A
  detached task queueing in a loop starves frame reading and the keep-alive tick for as long as it
  keeps going.

Both are reachable only from `detach` or from a handle the caller kept, both are the crate's own
escape hatches, and `max_lifetime` is the backstop. Still: an unbounded queue and a strictly biased
drain are a combination worth either bounding or documenting as a constraint on `detach`.

### 4.4 A panicking connection leaks its cancellation token

`Connection::finish` ends with `self.shutdown.cancel()` (`src/conn/connection.rs:617`), deliberately
last. On a panic, `catch_unwind` at `src/server.rs:698` catches the unwind and `finish` never runs,
so the token is dropped uncancelled -- and dropping a `CancellationToken` does not cancel it. Any
`detach`ed task awaiting `conn.shutdown().cancelled()` hangs until the server's `live` token fires
at shutdown. Narrow, but it is exactly the path that is hardest to notice.

*Fix:* a `DropGuard` on the connection's token, or cancel in the `Err` arm of the `catch_unwind`.

---

## 5. Two observations about the proposal as a document

**The design lives in doc comments, and a lot of it is argument rather than reference.** Modules
open with the case against the design they replaced -- "an earlier iteration generated ... from a
`packet!` macro", "the 470-line `listen()` it replaces", "an earlier revision routed every comparison
through a named gate". That is valuable *now*, while the proposal is being judged, and it is
technical debt the moment it is accepted: a reader in a year gets a rebuttal to code they have never
seen, in the place they went looking for what a function does. `REVIEW.md` is the right home for all
of it. The API documentation should say what things do.

**The prose is confident in a way the code sometimes is not.** "`on_error` is the one place that job
can live" (§4.1) and "each layer keeps its own books" (§4.2) both read as settled principles, and
both are doing work the implementation does not support. A design document that argues its own case
this well is harder to review, not easier -- which is worth knowing when the next round of findings
is written.

---

## 6. Recommendation

**Adopt the bottom half without reservation.** `wire`, `version`, `packet` and `codec` should replace
`passage-packets` regardless of what happens to the rest. They meet the versioning requirement,
they close real bug classes by construction, and they carry almost no framework cost.

**Adopt the top half, with four changes first:**

1. Re-add the end-of-connection hook with `log_ending`'s own signature (§4.2). Without it the
   telemetry requirement is unmet and the tests assert on log prose.
2. Fix the `on_error`/task-drop contract -- documentation at minimum, ordering preferably (§4.1).
   Do this before the discovery adapter is ported, not after.
3. Document the conditional-layer pattern, or ship `Either` (§1.5). Passage's config makes this
   unavoidable on the first day of `passage-server`.
4. Cancel the connection token on panic (§4.4).

**Decide explicitly, rather than by omission:** compression (and what `Limits` means once it
exists), NBT, legacy ping, and whether `Phase::Play` is a real target or should stop being mentioned.

**Move the historical argument out of the doc comments and into `REVIEW.md`** before anything is
written on top of this.

The open question the proposal has not answered is §1.1: the handler model is the part that will be
hardest to reverse, it is justified mainly against a strawman (one big trait), and the sequencing it
gives up is re-implemented by hand in the demo. It is probably still the right call -- keep-alives
overlapping backend selection is the case that breaks the linear version, and it is Passage's actual
flow. But it should be adopted because that case was weighed, not because the framework is the part
that got built.
