# `passage-driver` → `passage-core`: migration comparison

A file-by-file comparison of the agent-generated `passage-driver` sketch against the migrated
`passage-core` crate, followed by a critical evaluation of what the migration gained and what it
cost.

**Method.** Every source file of both crates was read. Claims marked **verified** were checked by
compiling `passage-core` and by driving its public API from a throwaway external crate; the exact
observations are quoted inline. Claims marked *by inspection* were not executed.

**Baseline.** `passage-driver`: 5 926 lines of `src/` (984 of them the optional `demo` feature),
2 175 lines of integration tests, 80 test functions, all passing. `passage-core`: 3 304 lines of
`src/`, 1 test function (the `cargo new` template's `add(2, 2)`).

---

## Status

Every decision recorded in the blockquotes below has been applied. What that leaves:

* **Fixed:** §3.1 (frame IDs, both directions), §3.2 (`Server::new` requires the listener), §3.3
  (types re-exported at the crate root), §3.4 (`on_version` forwarded; the `Arc` impl removed,
  because `&mut self` cannot be forwarded through one), §3.5 (dead error variants removed; the
  overwrite-with-warning is now documented behaviour), §3.6 (`MAX_PACKET_ID`; out-of-range IDs are
  skipped with a warning instead of sizing the table), §3.8 (`settle` replaces the second `drive`, so
  no frame is dispatched after the connection has failed), §3.9 (the close gets its own token),
  §3.11 (log branches inverted back), §3.12 in full, §5.1 (`DispatchError` carries `class` and
  `label`; unknown packets are `Class::Peer` again), §5.4 (`can_reply` re-added). `#![deny(unsafe_code)]`
  and `#![warn(missing_docs)]` are back, and the crate is clippy- and rustfmt-clean.
* **Accepted as designed, not changed:** §3.7 (a handler that answers an error itself must not also
  fail), §3.10 (cancelling the server cancels its connections), §5.2 (no cross-phase packet lookup),
  §5.3 (`anyhow` as the handler source type), §5.5 (`tick_interval: None` is the opt-out), §5.6
  (`Ctx` has no forwarding methods, so the version is visible at every send).
* **Outstanding:** §4. The test suite is still to be ported.

The sections below are the original findings and are left unedited, so the decisions read against
what they answered.

---

## 1. Structural map

| `passage-driver`                        | `passage-core`                                                     | Note                              |
|-----------------------------------------|--------------------------------------------------------------------|-----------------------------------|
| `src/lib.rs` (crate docs, `#![warn(missing_docs)]`) | `src/lib.rs`                                       | docs and lints dropped; template `add()` + test still present |
| `src/version.rs`                        | `src/version.rs`                                                    | private module                    |
| `src/packet.rs`                         | `src/packet.rs` + `src/phase.rs` + `src/direction.rs`               | split; all three private          |
| `src/wire.rs`                           | `src/wire/{mod,options,reader,writer,error}.rs`                     | split                             |
| `src/codec.rs`                          | `src/codec/{mod,codec,cipher,error}.rs`                             | split                             |
| `src/router.rs`                         | `src/router/{mod,router,table,dispatch,error}.rs`                   | split                             |
| `src/conn/{mod,connection,dispatch,handle}.rs` | `src/connection/{mod,connection,dispatch,handle,error}.rs`   | renamed + split                   |
| `src/server.rs`                         | `src/server/{mod,server,listener,layer,error}.rs`                   | split; `error.rs` is empty        |
| `src/error.rs` (one taxonomy)           | four per-module error enums                                         | see §3.1                          |
| `src/demo/**` (feature `demo`)          | —                                                                   | dropped                           |
| `tests/**` (4 files, 2 175 lines)       | —                                                                   | dropped                           |
| `README.md`, `REVIEW.md`, `ASSESSMENT.md`, `NOTES.md` | —                                                     | dropped                           |
| —                                       | `src/client/{mod,client}.rs`                                        | new, empty (0 bytes)              |
| deps: `thiserror`                       | deps: `thiserror` + **`anyhow`**                                    | see §5.3                          |

The split itself is the clearest win of the migration and is not in question below. Everything that
follows is about what changed *inside* the modules.

---

## 2. What the migration genuinely improved

1. **Module decomposition.** `wire.rs` at 786 lines and `server.rs` at 792 were the two files most
   in need of splitting, and both split along honest seams (reader / writer / options / error;
   listener / layer / server).

2. **Per-module error types.** `wire::WireError` knows nothing about phases, versions or packets —
   it is a pure parsing error. The driver's single `Error` forced `ProtocolError` to carry
   `UnknownPacket { phase, version, id }` next to `Utf8 { field }`. Layering the errors per module is
   the better factoring.

3. **Field names in every wire error.** The driver's `ProtocolError::Eof`, `VarIntTooLong` and
   `VarIntNotCanonical` said *what* failed but not *where*: `Reader::take`, `u8`, `var_int` had no
   field parameter. `passage-core` threads `field: &'static str` through all of them
   (`wire/reader.rs`). This is a real diagnostic improvement and the single best change in the crate.

4. **`Reader::take` / `Reader::fixed` are public.** A codec outside the crate can now read a
   fixed-width field the crate does not know about. In the driver both were private.

5. **The `Wire` trait is gone.** `Reader::array` and `Writer::array` now take a closure instead of
   requiring `T: Wire`. The driver could not decode a `Vec<String>` without a newtype; the closure
   form handles primitives and composites alike with one fewer public trait. Good simplification.

6. **`ProtocolVersion: From<i32> + AsRef<i32>`**, and `Options::permissive()` — small ergonomic
   additions with no downside.

7. **`check_ids_unordered` lives next to `ids`** (`packet.rs`) rather than in `router.rs`. The
   invariant and its check are now in the same file as the thing they constrain.

8. **`Dispatcher` has default method bodies**, plus impls for `()`, `Box`, `Arc` and `Option`. A
   dispatcher that only wants `on_frame` no longer writes four methods. Lower barrier than the
   driver's fully-required trait.

9. **`ConnectionHandle` no longer carries a protocol version.** The driver re-stamped its handle on
   every `Op::SetVersion` (`at_version`) so that a handle captured into a background task would not
   encode against a stale version. `passage-core` takes the version as an argument to `send`, which
   removes the re-stamping machinery and the whole class of "which handle is current" question. The
   cost is verbosity at call sites; the trade is defensible.

10. **`ConnectionBuilder` gained `.dispatcher()` / `.state()` setters**, matching `Server`'s shape
    instead of the driver's three positional arguments to `Connection::builder`.

11. **`Frame` unifies `Frame` + `Encoded`.** One type for both directions is the right idea — the
    driver had two near-identical structs. The execution is broken (§3.2), but the concept is sound.

---

## 3. Defects introduced

### 3.1 The frame codec does not write packet IDs, and hands decoders the ID byte — **critical, verified**

`codec/codec.rs`. `Frame::of` resolves the packet ID into `Frame.id` and encodes *only* the payload
into `Frame.payload` (line 43–44 — no `writer.var_int(id)`). `Encoder::encode` then writes
`length(payload.len())` followed by `raw(&payload)` (lines 173–174). The ID is never emitted.

Symmetrically, `Decoder::decode` reads the ID varint from a `Reader` and then does
`let payload = frame.freeze()` (line 157) — the driver had `frame.split_off(id_len).freeze()`. The
ID bytes stay in the payload, so every `P::decode` is fed one extra leading byte and
`Reader::finish` will reject the packet as `TrailingBytes` (or silently misparse).

Verified by round-tripping a `Frame` through `FrameCodec` from an external crate:

```
input:   Frame { id: 0x42, payload: b"hello" }
wire:    [5, 104, 101, 108, 108, 111]          // length=5, then "hello" — no 0x42
decoded: id = 0x68 ('h'), payload = [104, 101, 108, 108, 111]
```

No packet can be sent or received. Both halves are one-line fixes, but nothing in the crate would
have caught them: this exact round trip was `frames_roundtrip` in the driver
(`passage-driver/src/codec.rs:268`), one of the 80 tests that were dropped.

> Decision: The packets should write and read their IDs. This should make the code more readable while
> keeping it fast. The ID is read double, once into the frame for routing and a second time by the packet
> decoding, but I would deem that okay.

### 3.2 The `Server` builder cannot be used — **critical, verified**

`server/server.rs:71`. Every setter, including `listener`, lives in
`impl<L: Listener, F, M, A: Layer<L::Io, L::Addr>> Server<L, F, M, A>`. `Server::default()` produces
`Server<(), (), (), ()>`, and `()` does not implement `Listener`, so the bound is unsatisfiable
before a listener is ever set:

```
error[E0599]: the method `listener` exists for struct `Server`,
              but its trait bounds were not satisfied
```

The driver kept the unconstrained setters in a bound-free `impl<L, F, M, A>` block and put only
`layer` — the one setter that genuinely needs `L::Addr` in scope — behind the bounds
(`passage-driver/src/server.rs:361` vs `:466`). Splitting the impl block the same way fixes it.

> Decision: Implementing listener for `()` should be fix it while keeping the simple code? Otherwise
> split as necessary.

### 3.3 The crate's core types are not publicly reachable — **critical, verified**

`lib.rs` declares `mod direction; mod packet; mod phase; mod version;` — private, with no
re-exports. But those modules hold `Packet`, `Phase`, `Direction` and `ProtocolVersion`, which appear
throughout the public API (`RouterBuilder::on<P: Packet>`, `ConnectionHandle::send(version, …)`,
`Options::initial_phase`, `Ctx::version`).

```
error[E0603]: module `version` is private
error[E0603]: module `phase` is private
error[E0603]: module `packet` is private
```

No downstream crate can define a packet, name a version, or set an initial phase. This does not warn
during `cargo check` because the items are `pub` inside a private module — it only surfaces at the
consumer. Four `pub mod` keywords.

> Decision: Make the types public but partially independent of the actual module structure.

### 3.4 `Box<dyn Dispatcher>` silently ignores version changes — **high, by inspection**

`connection/dispatch.rs:46`. The `Dispatcher for Box<D>` impl forwards `on_frame`, `on_tick` and
`on_error` — but not `on_version`, which now has a default no-op body. A boxed `RouterDispatcher`
therefore never rebinds its dispatch table and serves every connection from the
`ProtocolVersion::UNKNOWN` table for its whole life. The driver's `Box` impl forwarded all five
methods (`passage-driver/src/conn/dispatch.rs:94`) and `set_version` had no default body, so this
could not be forgotten.

`Arc<D>` and `Option<D>` have the same gap, though `on_version(&mut self)` cannot be forwarded
through an `Arc` at all — which is a sign the method does not belong on a trait that claims an `Arc`
impl.

> Decision: Implement the missing methods

### 3.5 Duplicate packet IDs are a warning instead of a build failure — **high**

`router/table.rs:74`. Two packets that resolve to the same `(phase, id)` now log
`warn!("Duplicate packet ID detected, overwriting previous")` and the later registration wins.
`RouterBuilder::build` is infallible. Consequences:

* A registration collision — the exact mistake a dispatch table exists to make impossible — becomes
  a line in a log file on a running server rather than a startup failure.
* `RouterError::IdCollision` and `RouterError::IdOutOfRange` are now **dead variants**: nothing in
  the crate constructs either (verified by grep). The error type advertises checks that no longer
  run.

> Decision: This is documented behavior. Remove the unused error types.

### 3.6 Unbounded allocation from a packet ID — **high, by inspection**

The driver rejected any resolved ID outside `0..=MAX_PACKET_ID` (1023) at build time
(`passage-driver/src/router.rs:278`). `passage-core` dropped both the constant and the check, and
`Table::new` does `let slot = id as usize; table.resize(slot + 1, None)` (`router/table.rs:70-73`).

A packet declaring `IDS: &[(V1_20_5, i32::MAX)]` makes `build()` attempt a ~4 GiB allocation; a
negative ID sign-extends through `as usize` to ~1.8×10¹⁹ and aborts. This is developer-controlled
data, not peer input, so it is a foot-gun rather than a vulnerability — but it turns a typo into an
OOM at startup instead of a named error.

> Decision: The max packet ID cannot be known as they are provided by another crate? If so, only pass
> a warning if the ID is unexpected.

### 3.7 Everything a failing handler queued is discarded — **high**

`connection/connection.rs:246-263`. `serve()` creates the handle/queue pair *inside itself*, and on
error creates a **second, fresh** pair for the error phase. The first receiver is dropped with
whatever was still in it.

The driver's contract was explicit: "a handler that queues a disconnect and *then* fails gets both",
implemented by `settle()` draining the existing queue before and after `on_error`
(`passage-driver/src/conn/connection.rs:582`). In `passage-core`, a handler that does
`ctx.handle.send(version, Disconnect { .. })?; Err(..)` sends nothing — the disconnect packet is
dropped on the floor and the peer sees a bare socket close.

> Decision: A handler that handles the error itself (i.e., sends a disconnect packet) should not also
> fail with an error. The on error hook is the last line of defense, not a control flow to handle errors.

### 3.8 The error path runs a full second connection loop — **high**

Same function. After the error hook, `serve()` calls `self.drive(&handle, &mut ops)` again. `drive`
only returns on close / cancellation / deadline / peer hangup, so unless `on_error` queues
`Op::Close`, every failed connection now:

* stays open for the whole `close_timeout` (5 s by default) instead of flushing and leaving;
* **keeps reading and dispatching frames from the peer** through the ordinary `on_frame` path, after
  the connection has already been declared failed;
* keeps firing tick handlers.

The driver's equivalent (`settle()`) was a `try_recv` drain that returned immediately and dispatched
nothing. Holding a failed connection open for five seconds while still accepting its input is both a
resource-exhaustion lever and a correctness hazard.

> Decision: We have to ensure that no other (detached) handler writes on the channel while the error
> handler sends packets. This was the only way I could think of while keeping the code slim.

### 3.9 The socket close is pre-cancelled on the error path — **medium**

`serve()` ends with `self.shutdown.cancel()` (line 276). `run()` then closes the socket via
`guarded(&self.shutdown, …)` (line 231), whose `select!` has an already-cancelled token in it. The
first `Poll::Pending` from `framed.close()` loses the race and the close is abandoned with
`debug!("failed to close the socket")`. Every connection that ends in an error skips its clean
shutdown.

> Decision: Maybe create a new token for that too? It should kept simple.

### 3.10 Graceful drain no longer exists — **medium**

`server/server.rs:254`: `let shutdown = self.shutdown.child_token();`. Connections are now children
of the server's own shutdown token, so cancelling the server cancels every live connection in the
same instant. `tasks.wait()` then has nothing to wait for.

The driver created an independent `live` token precisely so that "cancelling the server stops
accepts, not connections", with `CancelOnDrop` as the safety net
(`passage-driver/src/server.rs:532`). `CancelOnDrop` survived the migration verbatim but is never
constructed — the compiler reports it as dead code, which is the clearest single signal that this
was an accident rather than a decision.

The drain timeout is also now a no-op: on expiry it logs `warn!("the drain timeout expired")` and
returns without cancelling or waiting.

> Decision: This is by design. Cancelling the server should also stop any connections as soon as possible.
> The double wait is not preferred here.

### 3.11 Connection logging is inverted — **medium**

`server/server.rs:352`:

```rust
if !peer_error {
    debug!(?elapsed, ?version, ?phase, "connection closed");
} else {
    error!(?elapsed, ?version, ?phase, error = ?outcome.error, reason, "connection closed");
}
```

`is_peer_error()` is `true` for `Closed { Peer }` (an ordinary hangup) and `Closed { Timeout }`, and
`false` for `Codec` and `Dispatch` errors. So every client that closes its connection normally logs
at **error**, and every genuine internal failure logs at **debug**. A server-list ping — connect,
read MOTD, disconnect — produces an error-level line per scanner.

Both branches also carry the message `"connection closed"`, so the two cases are indistinguishable
by message.

> Decision: fix and update logging

### 3.12 Smaller correctness and hygiene items

| Location | Issue |
|---|---|
| `wire/options.rs:21` | `max_frame_len` default dropped from 32 KiB back to 8 KiB. The driver raised it deliberately with a regression test (`a_status_response_with_a_favicon_fits_the_default_frame`): a 64×64 favicon is base64 of a multi-KiB PNG, and the vanilla client caps status JSON at 32 767 characters. 8 KiB refuses ordinary MOTD content as our own oversized-frame error. |
| `wire/options.rs:7` | The doc for that field says 8 KB "exceeds any realistic frame size (strings are limited to `32767` bytes)" — the parenthetical contradicts the number it justifies. |
| `router/dispatch.rs:47` | `RouterDispatcher::new` hard-codes table index `0` instead of calling `router.table(UNKNOWN)`. Correct only while no packet declares an `IDS` entry below `UNKNOWN`; the driver computed it. |
| `router/dispatch.rs:17` | `table: (ProtocolVersion, usize)` stores the version but nothing ever reads it — the cache key is written and never compared. |
| `connection/connection.rs:294-300` | The op arm flushes *before* checking `ControlFlow::Break`, so a flush failure during `Op::Close` turns a clean close into an error. |
| `connection/connection.rs:306` | `exclusive.then(\|\| 1).unwrap_or(0)` — `usize::from(exclusive)` (clippy: `unnecessary_lazy_evaluations`). |
| `connection/error.rs:97` | `reason()` returns `Option<&'static str>` but every arm returns `Some`. |
| `connection/error.rs:6` | `use tracing::Level;` unused (compiler warning). |
| `connection/handle.rs:146` | `self.options.clone()` on a `Copy` type (clippy: `clone_on_copy`). |
| `server/error.rs` | Empty file; `pub use error::*;` in `server/mod.rs` warns as an unused import. |
| `server/server.rs:346` | `.map(..).flatten()` instead of `.and_then(..)`. |
| `router/mod.rs:9` | `pub use table::*` exports `Table`, `Entry` and `ErasedHandler` publicly, though every field is `pub(crate)` — the types are nameable but useless. |
| `wire/reader.rs:96-101` | `i8` has its `# Errors` section duplicated verbatim. |
| `wire/writer.rs:165` | `optional`'s doc links `[gated](Writer::gated)`, which does not exist, and claims "it does not write a length prefix" — neither statement applies to this method. |
| `lib.rs:12-25` | The `cargo new` template `add()` function and its test are still in the crate root. |
| `version.rs:80` | `// TODO move into router implementation where these are actually used` — and `V1_20_5`/`V26_2` are indeed dead (compiler warnings). `V1_21` was dropped. |
| `codec/codec.rs:40` | `// TODO try to reuse the same buffer …`, `wire/writer.rs:154` `// TODO use bytestring instead`, `connection/connection.rs:293` `// TODO handle errors better?` — three unresolved TODOs shipped into the migrated crate. |

> Decisions:
> - update max size if you are sure
> - protocol version will be positive so this is fine
> - fix control flow check
> - fix clippy warnings and unused code/imports
> - remove option from reason
> - update the lib root to export the types

---

## 4. Verification was removed, not replaced

| | driver | core |
|---|---|---|
| unit tests | 35 | 1 (template) |
| integration tests | 45 (2 175 lines) | 0 |
| doc examples compiled | yes (`no_run` in `lib.rs`, `server.rs`) | none |
| `#![warn(missing_docs)]` | yes | no |
| `#![deny(unsafe_code)]` | yes | no |
| runnable worked example | `src/demo/` behind a feature | none |

This is the reason §3.1 through §3.4 got as far as a committed state. Each of them was covered:

* §3.1 → `frames_roundtrip`, `partial_frames_are_not_an_error`,
  `encryption_applies_from_the_switchover_point_only` (`passage-driver/src/codec.rs`)
* §3.2 → every test in `passage-driver/tests/server.rs` builds a `Server`
* §3.3 → `passage-driver/src/demo/` is compiled as a *consumer* of the public API
* §3.4/§3.7/§3.8 → `passage-driver/tests/ending.rs`, `tests/dispatch.rs`

The integration tests are the highest-value thing to port, and they port almost unchanged: they
drive a `Connection` over a `tokio::io::duplex` pair with a test-double dispatcher, which is exactly
the seam `passage-core` kept.

---

## 5. Design decisions that were reversed — worth re-deciding deliberately

These are not bugs. They are places where the migration chose the opposite of a documented driver
decision, and the reasoning is not recorded anywhere in `passage-core`.

### 5.1 Blame classification is gone

The driver's `Class { Peer, Transport, Internal }` decided log level, metric label and whether to
report to Sentry, and `Error::peer(label, source)` / `Error::internal(label, source)` let a handler
classify its own failures. `passage-core` has `ConnectionError::is_peer_error()` — a boolean that
cannot distinguish "the peer sent garbage" from "our adapter is down", and which §3.11 then uses
backwards.

Given that the project's stated requirement is *"keep the telemetry and even expand upon it"*, this
is the load-bearing loss. `reason()` is a partial replacement but is only defined for the connection
layer's own variants; a `Dispatch(anyhow::Error)` collapses to the single label `"dispatch"`.

> Decision: we cannot list every error the custom handlers will use. As such we have to use something
> like anyhow. Maybe we could make the DispatchError a struct with additional metadata. But I'm unsure
> whether it would be filled by the custom implementation. Telemetry can still use the anyhow error
> downcast. You may propose a different solution

### 5.2 The typed unknown-packet diagnostic is gone

The driver's dispatcher looked an unroutable ID up in the *other* phases before failing, so a login
packet arriving in the status phase reported
`packet 'LoginStart' belongs to phase Login but arrived in phase Status` rather than "unknown packet
0x00". `passage-core` (`router/dispatch.rs:78`) replaces both with an `anyhow::bail!` string. The
`lookup_elsewhere` helper was deleted.

> Decision: packet Ids are reused. Looking at other tables might cause confusion.

### 5.3 `anyhow` in a library's public API

`DispatchError = anyhow::Error` (`connection/dispatch.rs:6`) and `ConnectionError::Dispatch(#[from]
anyhow::Error)`. The comment at `router/dispatch.rs:76-77` acknowledges this — *"While not ideal, we
send an 'anyhow' error from the library code here"* — which is worth taking at face value. It means
no caller can match on why a handler failed, and it is what forces §5.1's collapse.

An alternative that keeps the ergonomics: make the handler error generic or keep a small
`{ class, label, source: Box<dyn Error> }` shape as the driver did. `anyhow` also carries a
backtrace allocation on every error construction, which for peer-triggered rejections (the common
case in this protocol) is paid per scanner.

> Decision: You may do that, but think if there are better solutions.

### 5.4 `Ending` was folded back into the error type

The driver kept "why a connection stopped" (`Ending`) separate from "what a fallible operation
returns" (`Error`), and argued at length that `Err(ConnectionClosed)` for a normal hangup is how the
*previous* Passage implementation ended up treating `Ok` and `Err` identically at every call site.

`passage-core` reintroduces exactly that shape: `ConnectionError::Closed { reason: Peer | Timeout |
Shutdown }`, with `Outcome.error: Option<ConnectionError>`. It is a smaller type surface, and the
`&mut ConnectionError` in `on_error` (letting a handler rewrite the ending) is a genuine capability
the driver lacked. But §3.11's inverted logging is the first instance of the failure mode the driver
predicted — a caller that cannot tell the happy path from the sad one by looking at the type.

Also lost: `Ending::can_reply()`, which told `on_error` whether the peer was still there to receive a
disconnect message.

> Decision: Re-add the can_reply method. Keep the error type for now. Adding to many enum and struct
> types might be confusing. Also, the happy path is not included in the error type.

### 5.5 `Dispatcher::ticks()` was removed

The driver only armed `tokio::time::Interval` when the dispatcher actually had a tick handler
(`filter(|_| dispatcher.ticks())`). `passage-core` arms it whenever `tick_interval` is set, so a
router with no `on_tick` wakes every connection on a timer to call an empty method. Minor per
connection, measurable at 10 000.

> Decision: The caller will be able to keep the tick interval empty if they do not want to handle it, right?

### 5.6 `Ctx`'s convenience methods were removed

`ctx.send(p)` / `ctx.close()` / `ctx.batch(..)` are now `ctx.handle.send(ctx.version, p)` /
`ctx.handle.close()`. This is the "simplify types" change working as intended — one less forwarding
layer — but it does push the version-threading burden onto every call site, which is what §3 of the
driver's own `ASSESSMENT.md` called the thing most likely to be got wrong by hand.

> Decision: This is deliberate to ensure that the caller is reminded how the handler state might not
> match the actual connection state (at the time the packet is sent).
---

## 6. Documentation

The driver carried its design rationale in module docs — roughly 40 % of its `src/` bytes were
comments. `passage-core` keeps the API-level doc comments (and adds `# Errors` sections, which the
driver did not have) but drops essentially all of the *why*: the module-level essays on the
operation queue, the read gate, the table-per-breakpoint scheme, the layer seam and the error
taxonomy are gone, along with `README.md`, `REVIEW.md` and `ASSESSMENT.md`.

That is a defensible trade — the essays were long, and some were arguing against alternatives nobody
had proposed. But three specific pieces of that prose were load-bearing and their loss shows up as
concrete defects above:

* why `max_frame_len` is 32 KiB and not 8 (→ §3.12);
* why connections hang off an independent token rather than the server's (→ §3.10);
* why a failing handler's queued packets must still be written (→ §3.7).

Those belong in `passage-core` as short comments at the three lines in question, not as prose files.

A few `# Errors` sections also document the wrong error: `Reader::bytes` and `Reader::string` claim
only `Eof` but propagate `NegativeLength` and `LengthLimit` from `length`.

> Decision: Fix the wrong docs (keep it short!) and add a short doc on the lib and actually exported modules.

---

## 7. Assessment

The restructuring is right. The module split, the per-module error types, the field names in wire
errors, the removal of the `Wire` trait and the version-free `ConnectionHandle` are all improvements
that should be kept, and the crate is roughly a third smaller for it.

What the migration did not carry across is the *verification*. Eighty passing tests, a compiled
worked example and two lint attributes went to zero, and in their absence four defects landed that
each of those mechanisms would have caught independently — including a frame codec that cannot send
a packet ID and a builder that cannot be called. The crate compiles cleanly and is non-functional,
which is precisely the state a test suite exists to make impossible.

The design reversals in §5 are a separate matter and mostly legitimate simplifications, with one
exception: collapsing the error taxonomy into `anyhow` removes the blame classification that the
project's own telemetry requirement depends on, and §3.11 is what that looks like in practice.

### Suggested order

1. §3.1 frame ID (two lines) — nothing works without it.
2. §3.3 `pub mod` on the four private modules (four words).
3. §3.2 split the `Server` impl block.
4. **Port `passage-driver/tests/` and the `src/*` unit tests.** Do this before 5–9; they are what
   proves the rest.
5. §3.4 forward `on_version` through `Box`.
6. §3.7 + §3.8 + §3.9 — the error path. Replace the second `drive()` with a bounded drain.
7. §3.10 independent `live` token; wire up or delete `CancelOnDrop`.
8. §3.11 invert the log branches.
9. §3.5 + §3.6 restore the build-time ID checks; make `build()` fallible again.
10. §3.12 hygiene, the three TODOs, and the `add()` template.
11. Re-add `#![deny(unsafe_code)]` and `#![warn(missing_docs)]`.
12. Decide §5.1 / §5.3 deliberately and record the decision in a comment.

> Decision: Apply the decision above as necessary. The test will follow once I'm happy with the crate.
> You should re-add `#![deny(unsafe_code)]` and `#![warn(missing_docs)]`.
