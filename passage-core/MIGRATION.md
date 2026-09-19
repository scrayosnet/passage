# `passage-driver` → `passage-core`: migration comparison

A comparison of the agent-generated `passage-driver` sketch against the migrated `passage-core`
crate. Findings that have since been fixed are reduced to one line each in §3; what remains in full
is what was decided against. Nothing on the list is open.

**Baseline.** `passage-driver`: 5 926 lines of `src/` (984 of them the optional `demo` feature),
2 175 lines of integration tests, 80 test functions, all passing. `passage-core`: 6 682 lines of
`src/` including its unit tests, 3 078 lines of `tests/`, 176 test functions, all passing.

---

## 1. Structural map

| `passage-driver`                        | `passage-core`                                                     | Note                              |
|-----------------------------------------|--------------------------------------------------------------------|-----------------------------------|
| `src/lib.rs`                            | `src/lib.rs`                                                        | crate docs + lints restored; types re-exported here |
| `src/version.rs`                        | `src/version.rs`                                                    | private module, re-exported       |
| `src/packet.rs`                         | `src/packet.rs` + `src/phase.rs` + `src/direction.rs`               | split                             |
| `src/wire.rs`                           | `src/wire/{mod,options,reader,writer,error}.rs`                     | split                             |
| `src/codec.rs`                          | `src/codec/{mod,codec,cipher,error}.rs`                             | split                             |
| `src/router.rs`                         | `src/router/{mod,router,table,dispatch,layer,error}.rs`             | split; `Layer` moved here         |
| `src/conn/{mod,connection,dispatch,handle}.rs` | `src/connection/{mod,connection,dispatch,handle,error}.rs`   | renamed + split                   |
| `src/server.rs`                         | `src/server/{mod,server,listener,error}.rs`                         | split; `error.rs` still empty     |
| `src/error.rs` (one taxonomy)           | four per-module error enums                                         |                                   |
| `src/demo/**` (feature `demo`)          | —                                                                   | dropped                           |
| `tests/**` (4 files, 2 175 lines)       | `tests/{flow,bytes,server,dispatch,client}.rs` + `tests/common/`    | ported and regrouped, see §4      |
| `README.md`, `REVIEW.md`, `ASSESSMENT.md`, `NOTES.md` | —                                                     | dropped                           |
| —                                       | `src/client/{mod,client,connector,error}.rs`                        | new; the dialling half, see §7    |
| deps: `thiserror`                       | deps: `thiserror` + `anyhow`                                        | see §5.2                          |

---

## 2. What the migration improved

1. **Module decomposition.** `wire.rs` at 786 lines and `server.rs` at 792 were the two files most
   in need of splitting, and both split along honest seams.

2. **Per-module error types.** `wire::WireError` knows nothing about phases, versions or packets --
   it is a pure parsing error. The driver's single `Error` forced `ProtocolError` to carry
   `UnknownPacket { phase, version, id }` next to `Utf8 { field }`.

3. **Field names in every wire error.** The driver's `Eof`, `VarIntTooLong` and `VarIntNotCanonical`
   said *what* failed but not *where*. `passage-core` threads `field: &'static str` through all of
   them. The single best change in the crate.

4. **`Reader::take` / `Reader::fixed` are public.** A codec outside the crate can read a fixed-width
   field the crate does not know about.

5. **The `Wire` trait is gone.** `array` takes a closure instead of requiring `T: Wire`. The driver
   could not decode a `Vec<String>` without a newtype.

6. **`ProtocolVersion: From<i32> + AsRef<i32>`**, and `Options::permissive()`.

7. **`check_ids_unordered` lives next to `ids`**, so the invariant and its check are in one file.

8. **`Dispatcher` has default method bodies**, plus impls for `()`, `Box` and `Option`. An
   implementation writes only the hooks it cares about.

9. **`ConnectionHandle` no longer carries a protocol version.** The driver re-stamped its handle on
   every `Op::SetVersion` so a captured handle would not encode against a stale one. Taking the
   version as an argument to `send` removes that machinery and the question it answered.

10. **`ConnectionBuilder` gained `.dispatcher()` / `.state()` setters**, matching `Server`.

11. **`Frame` unifies the driver's `Frame` + `Encoded`.** One type for both directions.

---

## 3. Resolved

Fixed and verified; the analysis that produced them is not repeated.

| # | Finding | Resolution |
|---|---------|------------|
| 3.1 | The frame codec wrote no packet ID, and left the ID in the inbound payload | `Frame::of` writes the ID varint ahead of the payload; `decode` leaves it in. The payload has the same shape in both directions and the router reads the ID once more before `P::decode`. No packet writes its own ID. |
| 3.2 | `Server::default().listener(..)` did not compile | `Server::default` removed; `Server::new(listener)` requires the listener up front, so the bounded impl block is satisfiable from the first call. |
| 3.3 | `Packet`, `Phase`, `ProtocolVersion` were unreachable from outside | Re-exported at the crate root, independent of the file layout. |
| 3.4 | `Box<dyn Dispatcher>` did not forward `on_version` | Forwarded through `Box` and `Option`. The `Arc<D>` impl was removed: `&mut self` cannot be forwarded through an `Arc`, so it could only ever have been the same silent bug. |
| 3.5 | ID collisions logged a warning while `RouterError` advertised checks that no longer ran | Overwrite-with-warning documented on `Table::new` and in `RouterError`; the dead `IdCollision` and `IdOutOfRange` variants removed. |
| 3.6 | `id as usize` reached `Vec::resize` unbounded | `MAX_PACKET_ID = 1023`. An out-of-range ID is warned and **skipped**, so it cannot size the table. |
| 3.8 | The error path ran a second full connection loop, dispatching peer frames for the whole `close_timeout` | `settle()` replaces the second `drive()`: it drains the hook's queue and flushes, and reads nothing from the peer. The fresh handle and queue are kept -- that isolation was the point. |
| 3.9 | The socket close raced an already-cancelled token and was abandoned | The close runs under a token of its own, bounded by the `close_timeout` lifetime. |
| 3.11 | Log levels were inverted: ordinary hangups at `error`, internal failures at `debug` | Peer errors at `debug` with `reason`, ours at `error` with `reason` and `cause`, clean close at `debug`. |
| 3.12 | Hygiene: 8 KiB frame limit, duplicated and wrong doc sections, `Option` return from `reason()`, unused imports, dead `CancelOnDrop`, clippy lints, `add()` template | All applied. `max_frame_len` is back to 32 KiB with the favicon reasoning recorded at the field. `#![deny(unsafe_code)]` and `#![warn(missing_docs)]` are back; the crate is clippy- and rustfmt-clean. |
| 5.1 | Blame classification was lost | `DispatchError` is now `{ class: Class, label: &'static str, source: anyhow::Error }`, with `peer`/`internal` constructors and `From<anyhow::Error>` defaulting to `Internal`/`"dispatch"`. Unknown packets are `Class::Peer` again. |
| 5.4 | `Ending::can_reply` had no counterpart | `ConnectionError::can_reply()` re-added. The error type itself stays as it is. |

Two changes beyond what the findings asked for, both forced by the above:

* **`From<ConnectionError> for DispatchError`.** Without it `ctx.handle.close()?` inside a handler
  stops compiling, which worked before only because `DispatchError` *was* `anyhow::Error`. The
  conversion carries the existing classification across rather than flattening it.
* **`is_peer_error()` returns `true` for `Codec(Wire(_))`.** A malformed frame is the peer's doing; a
  broken socket (`Codec(Io(_))`) is not. Without the split, every peer sending garbage logs at
  `error`.

---

## 4. Verification, ported

| | driver | core |
|---|---|---|
| unit tests | 35 | 121, one module at a time |
| integration tests | 45 (2 175 lines) | 55 across five binaries |
| runnable worked example | `src/demo/`, behind a feature | the test protocol in `tests/common/packets.rs` |

Every property the driver's tests proved is proved here, and the driver-specific ones were rewritten
rather than dropped: what a failing handler queued is now asserted to be *discarded* (§5.1), and
cancelling the server is asserted to end its connections (§5.3).

### Where the tests live

| File | What it holds |
|------|---------------|
| `src/**` | one `#[cfg(test)] mod tests` per module, against that module alone |
| `tests/flow.rs` | two routers meeting: versions, phases, gating, endings, deadlines |
| `tests/bytes.rs` | what a connection does with bytes that are not a packet |
| `tests/server.rs` | the accept loop: layers, state, shutdown, draining, panics |
| `tests/dispatch.rs` | a connection driven by a dispatcher that is not a router |
| `tests/client.rs` | the dialling half, including one exchange over a real TCP socket |
| `tests/common/` | the suite itself, below |

### The suite

The driver's tests each built a `Connection`, a duplex pair, a dispatcher and a hand-written client.
Here a test says what the two sides route, and nothing else:

```rust
let meeting = Scenario::new(server_router, client_router)
    .version(versions::V26_2)
    .run()
    .await;

meeting.expect_clean();
assert_eq!(meeting.client_saw(), ["status mc.justchunks.net"]);
```

`Scenario` runs both routers over a socket pair and returns **both** `Outcome`s, so an ending is
asserted as a value rather than fished out of a log. State is `Notes`, a shared list a handler
writes to, so "what did the server see" is a `Vec<String>`. Three seams sit beside it for what a
scenario cannot express: `Harness` (a real `Server` on a channel-backed listener), `RawClient` (a
peer that sends bytes rather than packets), and `record_logs` (for the server's endings, which are
reported by logging and by nothing else).

### What the port found

Three defects, each caught by the test that was written for a property rather than for the code:

| Found by | Defect | Fix |
|---|---|---|
| `a_hostile_length_prefix_is_a_peer_error_not_a_panic` | A payload that failed to decode was classified **internal** -- so any client sending a malformed packet would page somebody | The erased handler raises `Class::Peer` under `malformed_packet`, with the field the decoder named in the message |
| `a_panicking_connection_is_reported_loudly_rather_than_vanishing` | `panic_message(&payload)` coerced `&Box<dyn Any>` to a `dyn Any` holding the *box*, so every downcast missed and every panic logged the fallback text | `payload.as_ref()` |
| `spawned_work_runs_while_keep_alives_are_exchanged` | `Batch` could not change the version, so a handshake handler could not apply version and phase as one change | `Batch::set_version` |

---

## 5. Driver decisions deliberately not carried over

Not defects. Recorded so the reasoning is not rediscovered later.

### 5.1 A failing handler's queued packets are discarded

`serve()` gives the error hook a fresh handle and queue, so anything the failing handler had queued
is dropped. The driver drained it first, on the contract that "send this disconnect, then fail" has
to mean what it says.

**Decision:** a handler that answers an error itself must not also fail with one. `on_error` is the
last line of defence, not control flow.

### 5.2 `anyhow` as the handler source type

`DispatchError.source` is an `anyhow::Error`, because the driver cannot list what a custom handler
will fail with. The blame and the metric label are carried alongside it (§3, row 5.1), so telemetry
does not have to downcast; only the cause itself is opaque.

**Decision:** keep. The alternative (a generic error parameter on every handler) costs more at every
call site than it returns.

### 5.3 Cancelling the server cancels its connections

Connections hold a child of the server's shutdown token, so cancelling the server stops the accept
loop and every live connection at once. The driver used an independent token so that a connection
mid-transfer could still send its transfer packet, and waited for it.

**Decision:** by design; the double wait is not wanted. Each connection still gets its
`close_timeout` to say goodbye, and `drain_timeout` bounds how long the server waits for all of them.

### 5.4 No cross-phase lookup for an unrouted packet

The driver looked an unroutable ID up in the *other* phases before failing, so a login packet in the
status phase reported `packet 'LoginStart' belongs to phase Login` rather than "unknown packet 0x00".

**Decision:** packet IDs are reused across phases, so the other table's answer would mislead more
often than it helped. The error is a classified `Class::Peer` failure with the ID, phase and version.

### 5.5 No `Dispatcher::ticks()`

The driver only armed the timer when the dispatcher had a tick handler. `passage-core` arms it
whenever `tick_interval` is set.

**Decision:** `tick_interval: None` is the opt-out, and it is the caller's to set.

### 5.6 `Ctx` has no forwarding methods

`ctx.send(p)` is `ctx.handle.send(ctx.version, p)`.

**Decision:** deliberate. The explicit version reminds the caller that the handler's snapshot may not
match the connection's state by the time the packet is written.

---

## 6. Assessment

The restructuring was right. The module split, the per-module error types, the field names in wire
errors, the removal of the `Wire` trait and the version-free `ConnectionHandle` are all improvements,
and the crate is smaller for them.

What the migration did not carry across was the verification. Eighty passing tests, a compiled
worked example and two lint attributes went to zero, and in their absence four defects landed that
each of those mechanisms would have caught independently -- including a frame codec that could not
send a packet ID and a builder that could not be called.

That is now closed: 176 tests, every module unit-tested and every property the driver proved proved
again (§4). Porting them found three more defects, which is the argument for having done it.

---

## 7. New in `passage-core`: the client half

The driver had no client. `Client` is `Server` with the arrow turned around: the same builder, the
same `Layer` stack, the same state factory, the same `Router` -- only the socket comes from a
`Connector` rather than a `Listener`, and there is exactly one of it.

```rust
let outcome = Client::new(addr)
    .state(|_: &SocketAddr| ())
    .dispatch(Arc::new(router))
    .initial_version(versions::V26_2)
    .connect()
    .await?;
```

Three things follow from a client rather than a server owning the connection, and each of them
changed something outside `client/`:

1. **The dialling side speaks first.** Nothing in the crate could run before the first frame arrived,
   so a client had no way to send a handshake. `Dispatcher::on_open` is now called once, before the
   loop, with the handle in hand; `RouterBuilder::on_open` registers one on a router. The server
   ignores it by default, which is the correct behaviour for a side that answers rather than asks.

2. **A client already knows its protocol version.** It therefore never queues `Op::SetVersion`, and
   `RouterDispatcher` used to bind its table only on that operation -- so a connection configured
   with `initial_version` dispatched the whole session against the fallback table. `on_open` now
   binds it. That was a latent bug on the server side too, for anyone setting `initial_version`.

3. **One connection has a caller waiting for it.** `connect` returns the `Outcome` rather than
   logging it, because the caller is the one place that knows whether a failure matters. Only the
   two ways there is no connection at all -- the dial failed, a layer rejected it -- are a
   `ClientError`.

`Connected` is a `Connector` over a socket somebody else already opened, which is what makes this
the test harness: both halves of a `tokio::io::duplex` pair become two clients pointed at each
other, with no listener, no port and no `tokio::spawn` for the accept loop. `MakeDispatcher` is now
implemented for `()` as well, so a connection that only watches what the peer does needs no
dispatcher at all.
