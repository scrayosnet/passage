# 7. The reference implementation

Everything recommended in documents 2-5 is implemented in this crate and covered by tests, so the
proposals can be judged by reading code rather than prose. 31 tests pass; `cargo clippy
--all-targets` is clean.

```text
src/version.rs      ProtocolVersion, the version table, named Feature gates
src/wire.rs         Reader/Writer with Limits, the Wire trait, hostile-input tests
src/packet.rs       Packet trait, Phase, Direction, the packet! macro
src/codec.rs        FrameCodec: length prefix, packet id, pluggable Cipher
src/error.rs        ProtocolError / InternalError / Error, Class, labels
src/flow.rs         Flow (sync-or-async) and Update (deferred state change)
src/conn.rs         Op queue, ConnHandle, Ctx
src/router.rs       typed registration, per-version dispatch table, validate()
src/driver.rs       the loop, Completion, log_completion
src/demo/packets.rs a worked packet set (handshake, status, login, configuration)
src/demo/server.rs  the Passage flow as handlers over a Session
tests/flow.rs       end-to-end tests against a raw protocol client
```

`src/hooks.rs` is the earlier sketch. It is no longer part of the crate (not declared in `lib.rs`) and
is kept only for side-by-side comparison; document 3 quotes it.

## Walkthrough

### Declaring a packet

```rust
packet! {
    /// The authenticated profile.
    pub struct LoginSuccess {
        pub user_id: Uuid,
        pub user_name: String,
        pub properties: Vec<Property>,
        /// The session id. Only on the wire since the 26.2 protocol.
        pub session_id: Option<Uuid> = since(LoginSuccessSessionId),
    }
    phase = Login;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x02 ];
}
```

This generates the struct, `Packet::id`, a decoder that ends in `reader.finish(NAME)?`, and an encoder
that refuses to emit a packet missing a field its version requires. `Property` is a hand-written
`Wire` impl -- composite types that are not packets stay ordinary code.

### Registering handlers

```rust
pub fn router() -> Router<Session> {
    Router::new(Direction::Serverbound)
        .unknown(UnknownPolicy::Reject)
        .on::<Intention, _>(on_intention)
        .on::<StatusRequest, _>(on_status_request)
        .on::<PingRequest, _>(on_ping_request)
        .on::<LoginStart, _>(on_login_start)
        .on::<LoginAcknowledged, _>(on_login_acknowledged)
        .on::<KeepAliveResponse, _>(on_keep_alive_response)
        .on_tick(on_tick)
}
```

### A synchronous handler

```rust
fn on_ping_request(ctx: Ctx<'_, Session>, packet: PingRequest) -> Outcome<Session> {
    Flow::from_result(
        ctx.send(&PongResponse { payload: packet.payload })
            .and_then(|()| ctx.conn.close()),
    )
}
```

No allocation, no lock, no await. `close()` is a normal completion: the driver flushes and returns
`Completion::Closed`.

### An asynchronous handler

```rust
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Outcome<Session> {
    let conn = ctx.conn.clone();
    let version = ctx.version();
    let claimed = packet.user_name.clone();

    Flow::later(async move {
        let (name, id) = authenticate(&claimed).await?;
        conn.send(&LoginSuccess {
            user_id: id,
            user_name: name.clone(),
            properties: vec![/* ... */],
            session_id: version.has(Feature::LoginSuccessSessionId).then(Uuid::new_v4),
        })?;
        Ok(Update::apply(move |session: &mut Session| {
            session.profile = Some((name, id));
        }))
    })
}
```

While this future is pending the driver reads nothing from this connection, so no packet can be
dispatched into a half-authenticated session. The state change lands with exclusive access before the
next frame.

### Concurrency where it is actually needed

```rust
fn on_login_acknowledged(ctx: Ctx<'_, Session>, _packet: LoginAcknowledged) -> Outcome<Session> {
    ctx.conn.set_phase(Phase::Configuration);

    let conn = ctx.conn.clone();
    Flow::from_result(ctx.conn.detach(async move {
        let target = select_backend().await?;                 // may take seconds
        conn.send(&Transfer { host: target.0, port: VarInt(target.1) })?;
        conn.close()?;
        Ok(Update::none())
    }))
}
```

Backend selection runs alongside the keep-alive ticks; it is cancelled with the connection, and a
panic inside it becomes a `Class::Internal` error rather than a lost task.

## What the tests prove

| Property                                                        | Test                                                                 |
|-----------------------------------------------------------------|----------------------------------------------------------------------|
| One packet type serves several versions, both directions         | `demo::packets::tests::one_type_serves_both_versions`                 |
| A gated field changes the wire format by exactly its size        | `demo::packets::tests::the_gated_field_changes_the_encoding`          |
| Encoding a version-required field that is unset fails, not truncates | `demo::packets::tests::a_missing_required_field_fails_closed`      |
| "Packet does not exist in this version" is representable          | `demo::packets::tests::packets_report_the_versions_they_exist_in`     |
| The gated field follows the *client's* version end to end          | `flow::the_gated_field_follows_the_client_version`                    |
| A negative length prefix is a peer error, not a panic              | `wire::tests::negative_length_is_rejected_before_allocating`, `flow::a_hostile_length_prefix_is_a_peer_error_not_a_panic` |
| A huge length prefix never reaches an allocation                   | `wire::tests::huge_length_is_rejected_before_allocating`, `flow::an_oversized_frame_is_rejected_before_it_is_buffered` |
| Overlong and non-canonical varints are rejected                    | `wire::tests::var_int_rejects_overlong_encodings`                     |
| Ten-byte `VarLong`s round-trip                                     | `wire::tests::var_long_roundtrips_ten_byte_values`                    |
| Trailing bytes are rejected                                        | `wire::tests::trailing_bytes_are_reported`, `demo::packets::tests::trailing_bytes_are_rejected` |
| A partially received frame is backpressure, not an error            | `codec::tests::partial_frames_are_not_an_error`                        |
| Encryption applies from exactly the switchover point                | `codec::tests::encryption_applies_from_the_switchover_point_only`      |
| An async handler blocks further dispatch (ordering)                 | `flow::an_async_handler_blocks_further_dispatch`                       |
| Detached work coexists with ticks                                   | `flow::detached_work_runs_while_keep_alives_are_exchanged`             |
| An unknown packet ends the connection as a *peer* error             | `flow::an_unknown_packet_ends_the_connection_as_a_peer_error`          |
| A peer hang-up is a completion, not an error                        | `flow::a_peer_hangup_is_not_an_error`                                  |
| Cancellation ends the connection cleanly                            | `flow::cancellation_ends_the_connection_cleanly`                       |
| Old clients can ping but not log in                                 | `flow::logging_in_with_an_unsupported_version_is_refused`              |
| Conflicting packet IDs fail at bind time, not on first traffic       | `flow::the_router_rejects_conflicting_ids_at_bind_time`                |

## Deliberately not implemented

These are decisions to make, not oversights:

| Gap                                  | Why it is open                                                                                     |
|--------------------------------------|----------------------------------------------------------------------------------------------------|
| Bounded operation queue               | Changes `send` into a fallible-with-backpressure operation; see [04-runtime.md](04-runtime.md#known-limitation-the-operation-queue-is-unbounded) |
| Per-phase deadlines / packet budgets  | Needs a policy per phase, which belongs to the server layer                                         |
| Real AES-CFB8 `Cipher` impl           | Trivial to add (the existing code already has it); the trait exists so the driver stays dependency-free |
| Compression (`SetCompression`)        | Passage never enables it, but a general-purpose library eventually must; it is another codec layer below framing |
| `= until(Feature)` for removed fields | No case for it yet; the shape mirrors `since` exactly                                              |
| Fuzz target                           | Highest-value next step; the decode path is already a pure function                                 |
| Metrics and spans in the driver        | Designed in [06-layering-and-telemetry.md](06-layering-and-telemetry.md); the hooks (`Encoded.name`, entry names) are in place |
| A client flow                          | `Router::new(Direction::Clientbound)` is supported and the test harness is effectively one          |

## Suggested migration path

1. **Land the wire layer.** `wire.rs` plus its tests can replace `passage-packets::{reader,writer}`
   on its own and fixes the four hostile-input defects immediately. This is the change with the best
   safety-to-risk ratio and needs no architectural commitment.
2. **Convert packet declarations to `packet!`**, one phase at a time, keeping `passage-protocol`
   working against the generated types. The 26.2 session ID becomes a gated field here.
3. **Introduce the driver behind the existing listener**, with a router that reproduces today's flow.
   `Connection::listen` is deleted at the end of this step, not the start.
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
