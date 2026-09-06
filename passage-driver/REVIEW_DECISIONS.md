- **`FrameCodec::new` does not infer its defaulted type parameter.** Hit while writing a probe: a
  bare `FrameCodec::new(limits)` is `E0283`, requiring `let c: FrameCodec = …` or a turbofish. Type
  defaults do not participate in inference for associated functions. A `FrameCodec::boxed(limits)`
  constructor, or making the boxed form the only public path, removes the papercut.
-> Ignore for now

- **`Encoded::of` does not check `P::DIRECTION`.** `src/codec.rs:71-99`. The router validates
  direction at registration (`src/router.rs:157-164`) but the send path has no equivalent, so a
  server can encode a serverbound packet. Passing the direction into `ConnectionHandle` would close
  it.
-> Ignore for now

- **Dead public accessors.** `Router::inbound` and `Router::unknown_policy`
  (`src/router.rs:313-323`) have no callers since `RouterDispatcher` reads the fields directly.
-> Remove unused

  **Solved** in the previous round, along with the speculative `breakpoints()` accessor.

- **`wire.rs` is missing primitives the real packet set needs**: NBT (the Configuration-phase
  `Disconnect` and `AddResourcePack` prompt are network-NBT text components,
  `passage-packets/src/writer.rs:105-116`), bool-prefixed optionals (`CookieResponse.payload`,
  `passage-packets/src/login.rs:404-423`), `i8`/`i16`/`i32` big-endian, `f32`/`f64`. Note that
  `Reader::gated` is *version*-gated (`src/wire.rs:268-278`) and is not the bool-prefixed optional —
  the naming invites confusing the two, so the optional wants its own method rather than a reuse.
  -> Implement these primitives, ignore NBT for now

  **Solved.** `i8`/`i16`/`i32`/`f32`/`f64` on both `Reader` and `Writer`, plus `u64` and `uuid`
  rewritten onto a shared `Reader::fixed::<N>()` so every fixed-width read is one bounds check in
  one place. The optional got its own pair, `Reader::optional` / `Writer::optional`, and the doc on
  each of the four methods says which of the two kinds it is: `optional` is the *peer's* choice and
  costs a byte even when absent, `gated` is the *version's* and costs nothing. NBT skipped.

- **`Phase` has three parallel definitions.** `COUNT`, `ALL` and `index()`
  (`src/packet.rs:52-75`); a new phase needs three edits and only a test ties them together
  (`:160-164`). `#[repr(usize)]` plus `COUNT = ALL.len()` and `index() = self as usize` removes two
  of the three.
-> Clean up

  **Solved** exactly as suggested. `ALL` is now the single definition — `COUNT = ALL.len()` and
  `index() = self as usize` under `#[repr(usize)]` — so adding a phase is a variant plus an entry.
  The existing test that ties the two together stays, because `ALL` in the wrong order would still
  be wrong and nothing else would say so.

- **`InternalError` does not derive `PartialEq`** while `ProtocolError` and `BuildError` do
  (`src/error.rs:39`, `:167`, `:353`), so tests match structurally on one and compare the others.
-> Implement `PartialEq`

  **Solved.** Every field is a `&'static str`, a `ProtocolVersion` or a `usize`, so the derive is
  free. `Error` itself still cannot have one (`io::Error` and `Box<dyn Error>`), which is why the
  tests that match on an `Ending` keep using `matches!`.

- **`Error::Closed` is classified `Class::Transport`** (`src/error.rs:305`) but documented as "not a
  failure of the connection … callers usually ignore it" (`:267-270`). A handler that `?`s it ends
  the connection as a transport error. Worth either its own class or a doc note that it is meant to
  be swallowed.
-> Fix if still a problem

  **Not a problem, and now documented as one that cannot arise.** Tracing who can actually observe
  it: `queue` returns `Closed` only when the receiving end is gone, and the receiver lives in the
  connection. A handler — and anything `spawn`ed or `exclusive` — is polled *by* the loop that owns
  it, so for them the queue can never be closed and `?` cannot produce this. The only observers are
  outside that task (`detach`, or a handle kept from `Connection::new`), and `detach` already drops
  it. A fourth `Class` would exist for a case that cannot reach a classifier; the doc on the variant
  now says that, so the next reader does not re-derive it.

- **`is_peer_error` matches `ConnectionRefused`** (`src/server.rs:283`), which `accept` does not
  produce. Harmless.
-> Ignore

- **Nobody sets `TCP_NODELAY`.** Neither implementation does (the old one only in its test harness,
  `passage-protocol/tests/support/mod.rs:109`). With the driver's batched flushes, Nagle can add
  ~40 ms to a status ping — the one latency users see. `serve` is the natural place.
-> Add to options/builder

  **Done, but not as a builder knob — flagging the deviation.** It is set in
  `impl Listener for TcpListener::accept`, unconditionally. Two reasons it did not become an option:
  `Server` is generic over `L::Io`, so it cannot reach a `TcpStream` to set anything, and
  `Listener::prepare` is an associated function with no `&self`, so a per-listener setting cannot
  reach it either. Making it configurable therefore means a new public listener wrapper type — and
  the option it would carry has no second useful value, since nothing about this protocol wants
  Nagle. A caller who disagrees writes a `Listener` impl, which is a dozen lines. Say the word and
  the wrapper goes in.

- **`docs/01`–`08` are stale across three rounds** — they still name `Driver`, `DriverConfig` and
  `ConnHandle`, do not know `serve`/`Listener`/`Dispatcher`/`RouterDispatcher` exist, `02` §B2 still
  describes named `Feature` gates, `04` still describes `Op::Send` as encoded at drain time, and the
  layer diagram in `docs/README.md` still says "Driver". Left alone per the standing instruction.
-> Ignore, removed

- **`Updates.md` is still in the tree** although it was folded into `docs/08-refinements.md`, and
  `NOTES.md` is staged for deletion.
-> Ignore, removed


---

Fix the Review.md C section as you see fit. Server side errors are less important than external clients.
So only update the code if it makes sense and does not introduce additional complexity.

## Section C, item by item

Read with the instruction in mind: a bug a *client* can walk into outweighs one only our own code
can, and nothing here is worth a new concept.

### C1 — the default frame limit. Fixed.

`Limits::max_frame_len` is 32 KiB (was 8 KiB, and 10,000 in the old implementation). The number is
not arbitrary: the vanilla client caps status JSON at 32,767 characters, so any limit below that
refuses content the client asks for by default — and refuses it as `InternalError::OversizedFrame`,
our own bug class, for a favicon. It is still 64× under vanilla's own inbound cap.

The demo's field limits came down to meet it, which was the other half of the finding: a
`MAX_STATUS_LEN` at or above the frame limit is not a limit at all, because the frame refuses first
and reports the wrong thing. They are 16 KiB and 8 KiB now, with a comment saying they have to leave
room for the rest of the frame. A test encodes a status response carrying a 12 KiB favicon under the
default limits, which is the concrete case that used to fail.

The suggested build-time check ("no packet's declared field limits exceed `max_frame_len`") is *not*
in. Those limits are constants inside hand-written decoder bodies; `build()` cannot see them without
adding them to the `Packet` trait, which is a new declaration on every packet to catch a mistake the
comment now warns about.

### C4 — a default config with no timeout. Fixed.

`max_lifetime` defaults to `Some(120s)`, matching what the old listener always applied. The
reasoning is that a default *is* the security posture: the documented minimal
`serve(listener, router, state).await` otherwise accepts sockets that idle forever, and holding one
costs a peer nothing. `None` still removes the cap, which is what anything reaching `Phase::Play`
wants — now a deliberate statement rather than what you get by not thinking about it. Test:
`a_connection_nobody_configured_still_has_a_deadline`.

`tick_interval` stays `None`: a tick with no handler behind it is already disarmed, and the bound
that matters is the lifetime.

### C5 — `MAX_PACKET_ID = 255`. Fixed, and it was never the width.

Raised to 1023 and re-documented. The table was never capped at 255 — it is sized by the IDs
actually registered in each phase (`table.resize(slot + 1, None)`), so the constant was only ever a
sanity ceiling on registration. It just sat below the Play phase, whose clientbound IDs run past
`0x80`. A server handling six packets pays nothing for the higher ceiling.

### C6 — `UNKNOWN == 0` overlaps a legal wire value. Fixed differently, and it found a real bug.

The suggested fixes were `Option<ProtocolVersion>` or a sentinel no client can send. Neither is
right, and chasing the sentinel turned up the actual defect: **inbound and outbound disagreed about
versions the table cannot place.**

`Router::table` placed a snapshot or a negative version on the floor table. `ids()` did not — it
compared numerically, so a snapshot (bit 30 set, above every release) resolved to the *newest* ID.
So for a snapshot, the ID we would accept for a packet was not the ID we would have sent for it; and
for a negative version, a status request was dispatched from the floor table and then could not be
answered at all, because no packet resolved — an `InternalError` raised by ordinary peer input,
which is precisely backwards.

The fix is one method, `ProtocolVersion::placed()`: itself when it can be ordered, `UNKNOWN` when it
cannot. Both `ids()` and `Router::table` go through it, so the rule is stated once and the two sides
cannot drift again. `table()` got shorter in the process — the `is_release` branch collapsed into
the binary search, because the floor is index 0 either way.

Consequence worth stating: a client sending garbage can now be *answered*. It gets a status response
(which is how it learns which version to install) and a login disconnect telling it why it was
turned away, because both of those packets are anchored at the floor. Tests:
`a_version_nothing_can_place_is_answered_rather_than_failing_internally`,
`a_login_at_a_version_nothing_can_place_is_told_why`, plus unit tests on `placed` and `ids`.

`UNKNOWN` stays `0`. With the placement rule the overlap is harmless by construction: a client that
sends `0` gets the floor, which is exactly what `UNKNOWN` means.

### C7 — `ids()` requires a descending table. Already fixed.

`BuildError::UnorderedIds`, added in the previous round; `RouterBuilder::on` checks at registration.

### C2 — per-field limits on writes. Not done.

This is our own outbound correctness — a `Transfer` with a 4 KiB hostname encodes and the *client*
rejects it — which is the side the instruction deprioritises. Closing it means a limit argument on
`Writer::string`/`bytes`, i.e. every encoder repeating the number its decoder already names, with
nothing checking that the two agree. That is more surface for a bug we cannot ship without a
hand-written decoder disagreeing with its own encoder first.

### C3 — the operation queue is unbounded. Not done.

The channel is fed only by our own handlers, never by peer input, so a peer cannot make it grow. The
scenario the finding described was a handler queueing sends into a socket blocked forever (A3) —
and A3 is fixed: every write is now raced against the lifetime and the shutdown token, so the
connection ends rather than accumulating. What is left is "a handler with an unbounded send loop
uses unbounded memory", which is true of any unbounded loop. A bounded queue would mean `try_send`,
a `QueueFull` error variant, and a decision at every call site about what to do when full — a new
concept for a case with no unprivileged trigger.

Both stay in `REVIEW.md`; neither is closed, they are deprioritised with a reason.
