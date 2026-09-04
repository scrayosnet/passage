# 7. The reference implementation

Everything recommended in documents 2-6 is implemented in this crate and covered by tests, so the
proposals can be judged by reading code rather than prose. 43 tests pass; `cargo clippy
--all-targets --all-features` is clean.

```text
src/version.rs      ProtocolVersion, the version table, named Feature gates
src/wire.rs         Reader/Writer with per-field limits, the Wire trait, hostile-input tests
src/packet.rs       Packet trait, ids(), Phase, Direction, AnyPacket
src/codec.rs        FrameCodec: length prefix, packet id, pluggable Cipher
src/error.rs        ProtocolError / InternalError / Error / BuildError, Class, labels
src/conn.rs         the Op vocabulary, ConnHandle, Ctx
src/router.rs       typed registration, per-version tables built once at startup
src/driver.rs       the loop, the read gate, deadlines, Completion
src/demo/packets.rs a worked packet set (handshake, status, login, configuration)
src/demo/server.rs  the Passage flow as handlers over a Session, plus log_completion
tests/flow.rs       end-to-end tests against a raw protocol client
```

The `demo` module sits behind a default-on `demo` feature: nothing in the driver depends on it, and
`default-features = false` keeps it out of a binary entirely.

[08-refinements.md](08-refinements.md) is the review that produced the current shape. It is worth
reading alongside this document, because three of its findings reversed earlier recommendations and
the reasoning that led to those is still worth having.

## Walkthrough

### Declaring a packet

```rust
pub struct LoginSuccess {
    pub user_id: Uuid,
    pub user_name: String,
    pub properties: Vec<Property>,
    /// The session id. Only on the wire since the 26.2 protocol.
    pub session_id: Option<Uuid>,
}

impl Packet for LoginSuccess {
    const NAME: &'static str = "LoginSuccess";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Clientbound;

    fn id(version: ProtocolVersion) -> Option<i32> {
        ids(version, &[(versions::V1_20_5, 0x02)])
    }

    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            user_id: r.uuid()?,
            user_name: r.string("user_name", MAX_NAME_LEN * 4)?,
            properties: r.array("properties", MAX_PROPERTIES, version)?,
            session_id: r.gated(version.has(Feature::LoginSuccessSessionId), Reader::uuid)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()> {
        w.uuid(&self.user_id);
        w.string(&self.user_name)?;
        w.array(&self.properties, version)?;
        if version.has(Feature::LoginSuccessSessionId) {
            let session_id = self.session_id.ok_or(InternalError::MissingField { .. })?;
            w.uuid(&session_id);
        }
        Ok(())
    }
}
```

Written out rather than generated -- see the note in
[02-versioning.md](02-versioning.md#a-note-on-how-the-codecs-got-written-the-packet-macro-tried-and-removed)
for why the `packet!` macro was removed. Three things visible here that the macro could not express:
each string names its own limit, the array names its own element bound, and the fail-closed decision
for the gated field is written where a reader will see it. `Property` is a `Wire` impl -- composite
types that are not packets stay ordinary code.

The trailing-bytes check is *not* here. The router calls `reader.finish(P::NAME)` after every decode,
so a hand-written codec cannot forget it.

### Registering handlers

```rust
pub fn router() -> Result<Router<Session>, BuildError> {
    Router::builder(Direction::Serverbound)
        .unknown(UnknownPolicy::Reject)
        .on::<Intention, _>(on_intention)
        .on::<StatusRequest, _>(on_status_request)
        .on::<PingRequest, _>(on_ping_request)
        .on::<LoginStart, _>(on_login_start)
        .on::<LoginAcknowledged, _>(on_login_acknowledged)
        .on::<KeepAliveResponse, _>(on_keep_alive_response)
        .on_tick(on_tick)
        .build(SUPPORTED_VERSIONS.iter().copied())
}
```

`build` resolves every ID at every supported version, so a collision is a startup failure. It is also
the last thing that can go wrong: `Driver::new` is infallible, and a connection takes an `Arc` of an
already-built table rather than constructing one.

### A handler

```rust
fn on_ping_request(ctx: Ctx<'_, Session>, packet: PingRequest) -> Result<()> {
    ctx.send(PongResponse { payload: packet.payload })?;
    ctx.close()
}
```

No allocation beyond the boxed packet, no lock, no await, no wrapper type. `close()` is queued behind
the pong, so the client gets its answer before the socket goes away, and the driver reports
`Completion::Closed` rather than an error.

### Reading state, and changing it

```rust
fn on_tick(ctx: Ctx<'_, Session>) -> Result<()> {
    if ctx.phase() != Phase::Configuration {
        return Ok(());
    }
    if ctx.state.awaiting_keep_alive.is_some() {
        // A client that stops answering is the *peer's* problem, and is classified as one.
        return Err(Error::peer("keep_alive_timeout", "the client missed a keep-alive"));
    }

    let id = ctx.state.next_keep_alive;
    ctx.update(move |session: &mut Session| {
        session.next_keep_alive += 1;
        session.awaiting_keep_alive = Some(id);
    })?;
    ctx.send(KeepAlive { id })
}
```

`ctx.state` is `&S`: reads are free and cannot see a half-applied change. Writes are an `Op::With`,
so they are ordered against the packet queued after them. Note also that this failure is
`Class::Peer` -- an earlier version reported a missed keep-alive as an internal error, which meant a
timing-out client was logged at `warn` and reported to Sentry.

### Waiting for something external

```rust
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Result<()> {
    let conn = ctx.conn.clone();
    let version = ctx.version();          // never changes after the handshake
    let claimed = packet.user_name;

    ctx.exclusive(async move {
        let (name, id) = authenticate(&claimed).await?;

        // Both are operations, so they land in exactly this order: the profile is recorded before
        // the packet that announces it reaches the wire.
        let profile = (name.clone(), id);
        conn.update(move |session: &mut Session| {
            session.claimed_name = claimed;
            session.profile = Some(profile);
        })?;

        conn.send(LoginSuccess {
            user_id: id,
            user_name: name,
            properties: vec![/* ... */],
            session_id: version.has(Feature::LoginSuccessSessionId).then(Uuid::new_v4),
        })
    })
}
```

`exclusive` states that the peer has nothing to send until this resolves. The driver keeps polling the
socket and reports a frame that arrives anyway as `EarlyPacket` -- and notices a hangup at once,
instead of after the authentication call returns on a connection nobody is on the other end of.

### Concurrency where it is actually needed

```rust
fn on_login_acknowledged(ctx: Ctx<'_, Session>, _packet: LoginAcknowledged) -> Result<()> {
    ctx.set_phase(Phase::Configuration)?;

    let conn = ctx.conn.clone();
    ctx.spawn(async move {
        let (host, port) = select_backend().await?;   // may take seconds
        conn.send(Transfer { host, port })?;
        conn.close()
    })
}
```

`spawn` rather than `exclusive`, because backend selection has to overlap with keep-alives -- the
client disconnects itself if the server goes quiet for 20 seconds. So the gate stays open, the tick
handler keeps running, and the task sends the transfer when it is done. It is cancelled with the
connection by being dropped.

## What the tests prove

| Property                                                             | Test                                                          |
|----------------------------------------------------------------------|---------------------------------------------------------------|
| Every packet round-trips in every supported version                   | `demo::packets::tests::every_packet_roundtrips_in_every_version` |
| A gated field changes the wire format by exactly its size             | `demo::packets::tests::the_gated_field_changes_the_encoding`   |
| Encoding a version-required field that is unset fails, not truncates  | `demo::packets::tests::a_missing_required_field_fails_closed`  |
| "Packet does not exist in this version" is representable               | `demo::packets::tests::packets_report_the_versions_they_exist_in` |
| An invalid enum value is a decode error, not a handler's problem       | `demo::packets::tests::an_invalid_intent_is_a_decode_error_not_a_handler_problem` |
| A per-field limit binds tighter than the frame                         | `demo::packets::tests::an_overlong_hostname_is_rejected_by_the_field_limit`, `wire::tests::a_per_field_limit_is_tighter_than_the_frame` |
| The gated field follows the *client's* version end to end              | `flow::the_gated_field_follows_the_client_version`             |
| A negative length prefix is a peer error, not a panic                  | `wire::tests::negative_length_is_rejected_before_allocating`, `flow::a_hostile_length_prefix_is_a_peer_error_not_a_panic` |
| A huge length prefix never reaches an allocation                       | `wire::tests::huge_length_is_rejected_before_allocating`, `flow::an_oversized_frame_is_rejected_before_it_is_buffered` |
| An unencodable length is refused instead of wrapping                   | `wire::tests::an_unencodable_length_is_refused_instead_of_wrapping`, `codec::tests::an_oversized_frame_is_refused_on_the_way_out_too` |
| Overlong and non-canonical varints are rejected                        | `wire::tests::var_int_rejects_overlong_encodings`              |
| Ten-byte `VarLong`s round-trip                                         | `wire::tests::var_long_roundtrips_ten_byte_values`             |
| Trailing bytes are rejected                                            | `wire::tests::trailing_bytes_are_reported`                     |
| A partially received frame is backpressure, not an error                | `codec::tests::partial_frames_are_not_an_error`                |
| Encryption applies from exactly the switchover point                   | `codec::tests::encryption_applies_from_the_switchover_point_only` |
| A packet sent during an exclusive task is reported, not replayed        | `flow::a_packet_sent_during_an_exclusive_task_is_a_protocol_error` |
| A hangup during an exclusive task ends the connection at once           | `flow::a_hangup_during_an_exclusive_task_ends_the_connection_at_once` |
| A well-behaved client completes the whole login flow                    | `flow::the_login_flow_completes_when_the_client_waits_its_turn` |
| State is recorded before the packet that announces it                   | `flow::state_is_recorded_before_the_packet_that_announces_it`   |
| Spawned work coexists with ticks                                        | `flow::spawned_work_runs_while_keep_alives_are_exchanged`       |
| An unknown packet ends the connection as a *peer* error                 | `flow::an_unknown_packet_ends_the_connection_as_a_peer_error`   |
| A packet from another phase is reported by name                         | `flow::a_packet_from_another_phase_says_so`                     |
| A peer hang-up is a completion, not an error                            | `flow::a_peer_hangup_is_not_an_error`                           |
| Cancellation ends the connection cleanly                                | `flow::cancellation_ends_the_connection_cleanly`                |
| An idle connection is dropped by its deadline                           | `flow::an_idle_connection_is_dropped_by_its_deadline`           |
| A lifetime deadline bounds even a chatty connection                     | `flow::a_lifetime_deadline_bounds_even_a_chatty_connection`     |
| Old clients can ping but not log in                                     | `flow::logging_in_with_an_unsupported_version_is_refused`       |
| Conflicting packet IDs fail at build time, not on first traffic          | `flow::the_router_rejects_conflicting_ids_at_build_time`        |
| A packet registered on the wrong-direction router fails at build time    | `flow::the_router_rejects_a_packet_travelling_the_wrong_way`    |

## Deliberately not implemented

These are decisions to make, not oversights:

| Gap                                  | Why it is open                                                                                     |
|--------------------------------------|----------------------------------------------------------------------------------------------------|
| Bounded operation queue               | Changes every `ConnHandle` method into a fallible-with-backpressure operation; see [04-runtime.md](04-runtime.md#known-limitation-the-operation-queue-is-unbounded) |
| Per-phase deadlines / packet budgets  | The two global deadlines cover denial of service; a per-phase policy belongs to the server layer    |
| Real AES-CFB8 `Cipher` impl           | Trivial to add (the existing code already has it); the trait exists so the driver stays dependency-free |
| Compression (`SetCompression`)        | Passage never enables it, but a general-purpose library eventually must; it is another codec layer below framing |
| Buffer reuse for outbound packets     | `Op::Send` allocates a `BytesMut` per packet at drain time. Batched flushing already removed the syscall per packet, which was the larger cost |
| Deduplicating identical version tables | Adjacent versions usually share an ID map, so twenty versions typically need three tables. Worth doing when the version list is long, not now |
| Golden-byte tests                     | Round-trip tests catch codec asymmetry but not a format change that stays symmetric                 |
| Fuzz target                            | Highest-value next step; the decode path is a pure function of `(version, phase, bytes)`            |
| Metrics and spans in the driver        | Designed in [06-layering-and-telemetry.md](06-layering-and-telemetry.md); the hooks (`Encoded.name`, entry names, the `exclusive` flag) are in place |
| A client flow                          | `Router::builder(Direction::Clientbound)` is supported and the test harness is effectively one       |

## Suggested migration path

1. **Land the wire layer.** `wire.rs` plus its tests can replace `passage-packets::{reader,writer}`
   on its own and fixes the four hostile-input defects immediately. This is the change with the best
   safety-to-risk ratio and needs no architectural commitment.
2. **Convert packet declarations** to hand-written `impl Packet`, one phase at a time, keeping
   `passage-protocol` working against the new types. The 26.2 session ID becomes a gated field here,
   and each field gets a real limit on the way past.
3. **Introduce the driver behind the existing listener**, with a router that reproduces today's flow.
   `Connection::listen` is deleted at the end of this step, not the start. Set `max_lifetime` and
   `max_idle` from the start -- they replace the existing coarse `connection_timeout`.
4. **Split the crates** as in [06-layering-and-telemetry.md](06-layering-and-telemetry.md) once the
   flow lives in handlers, because only then is the seam obvious.
5. **Add budgets, fuzzing and the metrics/spans** from documents 5 and 6.

Steps 1 and 2 are useful even if the driver design is rejected entirely.

## Note on the workspace build

`passage-protocol` currently does not compile: `src/connection.rs:81-91` contains an unfinished
`Visitor`/`Acceptor` experiment (`visitor.visit(ctx, self)` has its arguments swapped relative to the
trait). That predates this work -- it arrived with the `WIP` commit `5f85245`, alongside the driver
sketch -- and is unrelated to the driver crate, which builds and tests cleanly on its own
(`cargo test -p passage-driver`).
