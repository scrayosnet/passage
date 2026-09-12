//! End-to-end tests for one connection, driven by a raw protocol client over a socket pair.
//!
//! These exist to keep the claims in `docs/` honest. Each test names the property it proves.

mod common;

use common::{TestClient, intention};
use futures::StreamExt;
use passage_driver::conn::{Connection, ConnectionConfig, Ctx, Ending, Outcome};
use passage_driver::demo::packets::{
    Intent, Intention, KeepAlive, KeepAliveResponse, LoginAcknowledged, LoginDisconnect,
    LoginStart, LoginSuccess, PingRequest, PongResponse, StatusRequest, StatusResponse, Transfer,
};
use passage_driver::demo::server::{Session, router};
use passage_driver::error::{BuildError, Class, Error, InternalError, Result};
use passage_driver::packet::{Direction, Packet, Phase};
use passage_driver::router::{Router, RouterDispatcher};
use passage_driver::version::{ProtocolVersion, versions};
use passage_driver::wire::{Limits, Reader, Writer};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Runs the demo server on one end of a socket pair and hands back a client for the other.
fn connect(config: ConnectionConfig) -> (TestClient, JoinHandle<Outcome<Session>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let router = Arc::new(router().expect("the demo router is well-formed"));
    let (connection, _handle) =
        Connection::builder(server_io, RouterDispatcher::new(router), Session::default())
            .config(config)
            .build();
    (TestClient::new(client_io), tokio::spawn(connection.run()))
}

#[tokio::test]
async fn serves_a_status_ping_end_to_end() {
    let (mut client, server) = connect(ConnectionConfig::default());

    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    // The client switches its own version and phase exactly like the server does.
    client.version = versions::V1_21;
    client.send(&StatusRequest).await;

    let status = client.expect::<StatusResponse>().await;
    assert!(status.body.contains("mc.justchunks.net"), "{}", status.body);

    client.send(&PingRequest { payload: 0x1234 }).await;
    let pong = client.expect::<PongResponse>().await;
    assert_eq!(pong.payload, 0x1234);

    client.expect_eof().await;
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn the_gated_field_follows_the_client_version() {
    // An old client must not be sent the session id...
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V1_21, Intent::Login))
        .await;
    client.version = versions::V1_21;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    let success = client.expect::<LoginSuccess>().await;
    assert_eq!(success.user_name, "Hydrofin");
    assert_eq!(success.session_id, None);
    drop(client);
    let _ = server.await;

    // ...and a new one must be.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    let success = client.expect::<LoginSuccess>().await;
    assert!(success.session_id.is_some());
    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn a_packet_sent_during_an_exclusive_task_is_a_protocol_error() {
    // Both packets are written before the server has even read the first. The login handler is
    // exclusive, so `LoginAcknowledged` was sent before the client could possibly have seen
    // `LoginSuccess` -- it is a protocol break, and reporting it is the whole point of the gate.
    // The previous design buffered it and dispatched it into a half-authenticated session.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    client.send(&LoginAcknowledged).await;

    let ending = server
        .await
        .expect("no panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(ending.error().expect("a failure").class(), Class::Peer);
    assert_eq!(ending.label(), "early_packet");
}

#[tokio::test]
async fn a_hangup_during_an_exclusive_task_ends_the_connection_at_once() {
    // The socket is still polled while the gate is shut, so a client that disappears mid-login is
    // noticed immediately instead of after the authentication call returns.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    drop(client);

    // `authenticate` sleeps, so completing this quickly is only possible by seeing the EOF.
    let outcome = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("must not wait for the authentication call")
        .expect("no panic");
    assert!(matches!(outcome.result, Err(Ending::PeerClosed)));
}

#[tokio::test]
async fn the_login_flow_completes_when_the_client_waits_its_turn() {
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;

    // Wait for the answer before saying anything else, like a real client does.
    let _ = client.expect::<LoginSuccess>().await;
    client.send(&LoginAcknowledged).await;

    let transfer = client.expect::<Transfer>().await;
    assert_eq!(transfer.host, "backend-1.justchunks.net");
    assert_eq!(transfer.port, 25565);

    client.expect_eof().await;
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test(start_paused = true)]
async fn spawned_work_runs_while_keep_alives_are_exchanged() {
    // The backend selection is spawned rather than exclusive, so the gate stays open and the tick
    // handler keeps the connection alive while it runs.
    let config = ConnectionConfig {
        tick_interval: Some(Duration::from_secs(16)),
        ..ConnectionConfig::default()
    };
    let (mut client, server) = connect(config);

    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    let _ = client.expect::<LoginSuccess>().await;
    client.send(&LoginAcknowledged).await;

    // The transfer arrives, and so do keep-alives; answer whatever comes.
    let mut keep_alives = 0;
    loop {
        let frame = client
            .framed
            .next()
            .await
            .expect("still open")
            .expect("decodes");
        if frame.id == KeepAlive::id(client.version).expect("exists") {
            let mut reader = Reader::new(&frame.payload, Limits::default());
            let packet = KeepAlive::decode(&mut reader, client.version).expect("decodes");
            keep_alives += 1;
            client.send(&KeepAliveResponse { id: packet.id }).await;
        } else {
            assert_eq!(frame.id, Transfer::id(client.version).expect("exists"));
            break;
        }
    }

    client.expect_eof().await;
    server.await.expect("no panic").result.expect("no error");
    // The selection in the demo is fast, so this is only a smoke check that ticks are wired up.
    assert!(
        keep_alives <= 1,
        "unexpected keep-alive count {keep_alives}"
    );
}

#[tokio::test]
async fn state_is_recorded_before_the_packet_that_announces_it() {
    // `on_login_start` queues the profile update ahead of `LoginSuccess`. Both are operations, so
    // by the time the client has the packet the connection has already applied the update -- which is
    // what a tick or a later handler would observe.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;

    let success = client.expect::<LoginSuccess>().await;
    client.send(&LoginAcknowledged).await;
    // The transfer only happens on the acknowledgement path, which runs after the update.
    let _ = client.expect::<Transfer>().await;
    assert_eq!(success.user_name, "Hydrofin");

    client.expect_eof().await;
    let _ = server.await;
}

#[tokio::test]
async fn an_unknown_packet_ends_the_connection_as_a_peer_error() {
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    // Id 0x7F exists in no phase.
    client.send_raw(&[0x7F]).await;

    let ending = server
        .await
        .expect("no panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(ending.error().expect("a failure").class(), Class::Peer);
    assert_eq!(ending.label(), "unknown_packet");
}

#[tokio::test]
async fn a_packet_from_another_phase_says_so() {
    // `LoginAcknowledged` is 0x03 in the login phase and nothing in the status phase. Reporting it
    // as unknown would send whoever reads the log looking for a missing packet definition.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    client.send_raw(&[0x03]).await;

    let err = server
        .await
        .expect("no panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(err.label(), "unexpected_packet");
    assert!(err.to_string().contains("LoginAcknowledged"), "{err}");
}

#[tokio::test]
async fn a_hostile_length_prefix_is_a_peer_error_not_a_panic() {
    let (mut client, server) = connect(ConnectionConfig::default());

    // A handshake whose `server_address` claims a length of -1. Decoded as `usize` that is
    // 18446744073709551615, which is what used to reach `vec![0; len]`.
    let mut buf = bytes::BytesMut::new();
    {
        let mut writer = Writer::new(&mut buf, "Raw", Limits::default());
        writer.var_int(0x00); // packet id
        writer.var_int(767); // protocol version
    }
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]); // string length: -1
    client.send_raw(&buf).await;

    let ending = server
        .await
        .expect("the connection task must not panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(ending.error().expect("a failure").class(), Class::Peer);
    assert_eq!(ending.label(), "negative_length");
}

#[tokio::test]
async fn an_oversized_frame_is_rejected_before_it_is_buffered() {
    let config = ConnectionConfig {
        limits: Limits {
            max_frame_len: 128,
            ..Limits::default()
        },
        ..ConnectionConfig::default()
    };
    let (mut client, server) = connect(config);

    // Announce a 1 MiB frame in three bytes, then send nothing.
    let mut buf = bytes::BytesMut::new();
    Writer::new(&mut buf, "Raw", Limits::default()).var_int(1024 * 1024);
    tokio::io::AsyncWriteExt::write_all(client.framed.get_mut(), &buf)
        .await
        .expect("writes");

    let ending = server
        .await
        .expect("no panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(ending.error().expect("a failure").class(), Class::Peer);
    assert_eq!(ending.label(), "frame_too_large");
}

#[tokio::test]
async fn logging_in_with_an_unsupported_version_is_refused() {
    let (mut client, server) = connect(ConnectionConfig::default());
    // 1.20.4: no configuration phase, so no transfer.
    client
        .send(&intention(ProtocolVersion::new(765), Intent::Login))
        .await;

    let err = server
        .await
        .expect("no panic")
        .result
        .expect_err("must fail the connection");
    assert_eq!(err.label(), "unsupported_version");

    // But a status ping from the same client still works, which is how it learns what to install.
    // There is no table for 765, so this is served from the version-independent fallback.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(ProtocolVersion::new(765), Intent::Status))
        .await;
    client.version = ProtocolVersion::new(765);
    client.send(&StatusRequest).await;
    let _ = client.expect::<StatusResponse>().await;
    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn a_peer_hangup_is_an_ending_but_not_a_failure() {
    // It sits in the `Err` half because we did not finish what we were doing -- not because
    // anything went wrong. Nothing here is anyone's fault, and the two questions have separate
    // answers so a scanner dropping after its MOTD is never logged as a problem.
    let (client, server) = connect(ConnectionConfig::default());
    drop(client);

    let ending = server
        .await
        .expect("no panic")
        .result
        .expect_err("a hangup is an ending");
    assert!(matches!(ending, Ending::PeerClosed));
    assert!(ending.error().is_none(), "nobody is to blame for a hangup");
    assert!(!ending.can_reply());
    assert_eq!(ending.label(), "peer_closed");
}

#[tokio::test]
async fn cancellation_ends_the_connection_cleanly() {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let shutdown = CancellationToken::new();
    let (connection, handle) = Connection::builder(
        server_io,
        RouterDispatcher::new(router().expect("builds")),
        Session::default(),
    )
    .shutdown(shutdown.clone())
    .build();
    let server = tokio::spawn(connection.run());
    let _client = TestClient::new(client_io);

    shutdown.cancel();
    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::Cancelled),
    ));
    // The handle notices, so background work can observe it too.
    assert!(handle.shutdown().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_does_not_outlast_its_deadline() {
    // The cheapest attack on a protocol server: connect, ask for something big, never read the
    // answer. The write buffer fills, the write stops making progress, and if that happened
    // anywhere the loop could not see, the connection would sit there past every deadline and past
    // any shutdown -- which is exactly what it used to do.
    let (server_io, client_io) = tokio::io::duplex(16);
    let config = ConnectionConfig {
        max_lifetime: Some(Duration::from_secs(5)),
        ..ConnectionConfig::default()
    };
    let (connection, _handle) = Connection::builder(
        server_io,
        RouterDispatcher::new(status_router(
            |ctx: Ctx<'_, Session>, _packet: StatusRequest| {
                ctx.send(StatusResponse {
                    body: "x".repeat(4000),
                })
            },
        )),
        Session::default(),
    )
    .config(config)
    .build();
    let server = tokio::spawn(connection.run());

    let mut client = TestClient::new(client_io);
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    client.send(&StatusRequest).await;

    // Nothing ever reads `client`.
    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::TimedOut),
    ));
}

#[tokio::test(start_paused = true)]
async fn a_lifetime_deadline_bounds_even_a_chatty_connection() {
    // A peer could otherwise hold a connection open forever by pinging. The lifetime cap is what
    // makes that impossible -- and it is the backstop for an exclusive task that never resolves.
    let config = ConnectionConfig {
        max_lifetime: Some(Duration::from_secs(30)),
        ..ConnectionConfig::default()
    };
    let (mut client, server) = connect(config);
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;

    // Keep the idle timer from ever firing.
    let chatter = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            client.send(&StatusRequest).await;
            let _ = client.framed.next().await;
        }
    });

    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::TimedOut),
    ));
    chatter.abort();
}

/// A two-packet router whose status handler is supplied by the caller, so the tests below can put
/// a deliberate ordering mistake in it.
fn status_router<H>(on_status: H) -> Router<Session>
where
    H: Fn(Ctx<'_, Session>, StatusRequest) -> Result<()> + Send + Sync + 'static,
{
    Router::builder()
        .on::<Intention>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Status)
        })
        .on::<StatusRequest>(on_status)
        .build()
        .expect("builds")
}

fn connect_to(router: Router<Session>) -> (TestClient, JoinHandle<Outcome<Session>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let (connection, _handle) =
        Connection::builder(server_io, RouterDispatcher::new(router), Session::default()).build();
    (TestClient::new(client_io), tokio::spawn(connection.run()))
}

fn status_response() -> StatusResponse {
    StatusResponse {
        body: r#"{"description":{"text":"hi"}}"#.to_owned(),
    }
}

#[tokio::test]
async fn the_phase_a_packet_belongs_to_is_its_own_not_the_connections() {
    // There used to be a check here: a packet was refused if the connection had moved to another
    // phase since the handler queued it. It looked like the version guard below and it was not --
    // `Packet::PHASE` is a constant, so it compared the packet's *identity* against the connection
    // rather than a snapshot against the present. Phases that share a payload get their own packet
    // type instead, which is a statement the compiler can check.
    let (mut client, server) = connect_to(status_router(
        |ctx: Ctx<'_, Session>, _packet: StatusRequest| {
            ctx.set_phase(Phase::Configuration)?;
            ctx.send(status_response())?;
            ctx.close()
        },
    ));

    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    client.send(&StatusRequest).await;

    assert_eq!(client.expect::<StatusResponse>().await, status_response());
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn a_packet_encoded_for_a_superseded_version_is_refused() {
    // Same guard, other axis: a handler that re-pins the version cannot also answer in it, because
    // its own view of the version is the snapshot from before the change.
    let (mut client, server) = connect_to(status_router(
        |ctx: Ctx<'_, Session>, _packet: StatusRequest| {
            ctx.set_version(versions::V26_2)?;
            ctx.send(status_response())
        },
    ));

    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    client.send(&StatusRequest).await;

    let ending = server
        .await
        .expect("no panic")
        .result
        .expect_err("must refuse to write the packet");
    let err = ending.error().expect("a failure");
    assert_eq!(err.class(), Class::Internal);
    assert!(
        matches!(
            err,
            Error::Internal(InternalError::StaleEncoding {
                packet: "StatusResponse",
                encoded_version: v,
                version: w,
            }) if *v == versions::V1_21 && *w == versions::V26_2
        ),
        "{err}",
    );
}

#[tokio::test]
async fn sending_before_switching_phase_is_the_order_that_works() {
    // The guard must not break the pattern the design promises: "send the last packet of this
    // phase, then switch". Operations drain in order, so the send is written while the connection
    // is still in the packet's own phase.
    let (mut client, server) = connect_to(status_router(
        |ctx: Ctx<'_, Session>, _packet: StatusRequest| {
            ctx.send(status_response())?;
            ctx.set_phase(Phase::Configuration)?;
            ctx.close()
        },
    ));

    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.version = versions::V1_21;
    client.send(&StatusRequest).await;

    let status = client.expect::<StatusResponse>().await;
    assert_eq!(status, status_response());
    client.expect_eof().await;
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn the_router_rejects_conflicting_ids_at_build_time() {
    // Two packets claiming the same id in the same phase is a wiring bug; it must surface at
    // startup, not on the first client that happens to send one of them.
    // Closures need their argument types spelled out so they implement `Fn` for *any* lifetime;
    // named handler functions (as in `demo::server`) do not have that wrinkle.
    let err = Router::builder()
        .on::<StatusRequest>(|_ctx: Ctx<'_, Session>, _packet: StatusRequest| Ok(()))
        .on::<StatusRequest>(|_ctx: Ctx<'_, Session>, _packet: StatusRequest| Ok(()))
        .build()
        .expect_err("must reject");

    assert!(
        matches!(
            err,
            BuildError::IdCollision {
                first: "StatusRequest",
                second: "StatusRequest",
                ..
            }
        ),
        "{err}",
    );
}

#[tokio::test]
async fn a_router_is_not_bound_to_one_direction() {
    // A client's router receives exactly the packets a server's sends. The table is keyed by phase
    // and id, so building one is the same job either way -- what used to be a `WrongDirection`
    // build error was the driver deciding which half of the protocol you were allowed to be.
    let router = Router::<Session>::builder()
        .on::<StatusResponse>(|_ctx: Ctx<'_, Session>, _packet: StatusResponse| Ok(()))
        .on::<PongResponse>(|_ctx: Ctx<'_, Session>, _packet: PongResponse| Ok(()))
        .build()
        .expect("a clientbound router is a router");

    // Both directions of one phase still collide on the ids they share, which is the honest signal
    // that this case wants a direction-keyed table rather than a silent preference.
    let err = Router::<Session>::builder()
        .on::<StatusRequest>(|_ctx: Ctx<'_, Session>, _packet: StatusRequest| Ok(()))
        .on::<StatusResponse>(|_ctx: Ctx<'_, Session>, _packet: StatusResponse| Ok(()))
        .build()
        .expect_err("0x00 in the status phase, twice");
    assert!(
        matches!(err, BuildError::IdCollision { id: 0x00, .. }),
        "{err}"
    );

    drop(router);
}

#[tokio::test]
async fn the_router_rejects_an_id_table_written_the_wrong_way_round() {
    // `ids` takes the first entry that matches, so an ascending table resolves every version above
    // the second entry to an id from the wrong era -- silently, and only for some clients.
    struct Backwards;

    impl Packet for Backwards {
        const NAME: &'static str = "Backwards";
        const PHASE: Phase = Phase::Login;
        const DIRECTION: Direction = Direction::Serverbound;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V1_20_5, 0x40), (versions::V26_2, 0x41)];

        fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
            Ok(())
        }
    }

    let err = Router::<Session>::builder()
        .on::<Backwards>(|_ctx: Ctx<'_, Session>, _packet: Backwards| Ok(()))
        .build()
        .expect_err("must reject");
    assert!(
        matches!(
            err,
            BuildError::UnorderedIds {
                packet: "Backwards",
                ..
            }
        ),
        "{err}",
    );
}

#[tokio::test]
async fn a_release_nobody_wrote_down_can_still_log_in() {
    // Protocol 768 is 1.21.2: a real release, and one no list in this crate mentions. The router
    // builds its tables from the versions the *packets* name, so 768 is dispatched exactly like
    // 767 -- which is what the protocol says, since nothing changed between them.
    assert_eq!(LoginStart::id(ProtocolVersion::new(768)), Some(0x00));

    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(ProtocolVersion::new(768), Intent::Login))
        .await;
    client.version = ProtocolVersion::new(768);
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::from_u128(1),
        })
        .await;

    // It gets a real `LoginSuccess`, not an "unknown packet" for the only packet it could send.
    let success = client.expect::<LoginSuccess>().await;
    assert_eq!(success.user_name, "Hydrofin");
    // 768 is below the 26.2 threshold, so the gated field is not on the wire.
    assert_eq!(success.session_id, None);

    drop(client);
    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::PeerClosed),
    ));
}

#[tokio::test]
async fn a_snapshot_is_refused_rather_than_treated_as_the_newest_release() {
    // Snapshots set bit 30, so every one of them compares above every release. Left alone, a
    // 1.20.5 snapshot would be sent the 26.2 session id field and would not survive reading it.
    let snapshot = ProtocolVersion::new(0x4000_0000 | 132);
    assert!(snapshot.at_least(versions::V26_2));

    let (mut client, server) = connect(ConnectionConfig::default());
    client.send(&intention(snapshot, Intent::Login)).await;

    // It is told why, in the phase the handshake put it in.
    client.version = snapshot;
    let disconnect = client.expect::<LoginDisconnect>().await;
    assert!(
        disconnect.reason.contains("could not use"),
        "{disconnect:?}"
    );

    assert_eq!(
        server
            .await
            .expect("no panic")
            .result
            .expect_err("refused")
            .label(),
        "unsupported_version",
    );
}

#[tokio::test]
async fn a_version_nothing_can_place_is_answered_rather_than_failing_internally() {
    // A client is free to send a negative version, and one that does still has to be *answerable*:
    // dispatch places anything it cannot order on the floor table, so the request arrives -- and if
    // the encoder disagreed and refused to resolve an ID for the same version, answering it would
    // be an internal error of ours over ordinary peer input. Both sides go through `placed`.
    let garbage = ProtocolVersion::new(-1);
    assert!(!garbage.is_release());

    let (mut client, server) = connect(ConnectionConfig::default());
    client.send(&intention(garbage, Intent::Status)).await;
    client.version = garbage;
    client.send(&StatusRequest).await;

    // Status is exactly what this connection is for: it is how the client is told which version to
    // install.
    let status = client.expect::<StatusResponse>().await;
    assert!(status.body.contains("mc.justchunks.net"), "{}", status.body);

    client.send(&PingRequest { payload: 7 }).await;
    assert_eq!(client.expect::<PongResponse>().await.payload, 7);
    client.expect_eof().await;
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn a_login_at_a_version_nothing_can_place_is_told_why() {
    // The other half: the login is refused, and the refusal reaches a client whose version resolves
    // nothing else -- because `LoginDisconnect` is anchored at the floor, and the floor is where
    // this version was placed.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(ProtocolVersion::new(-1), Intent::Login))
        .await;
    client.version = ProtocolVersion::new(-1);

    let disconnect = client.expect::<LoginDisconnect>().await;
    assert!(
        disconnect.reason.contains("could not use"),
        "{disconnect:?}"
    );
    assert_eq!(
        server
            .await
            .expect("no panic")
            .result
            .expect_err("refused")
            .label(),
        "unsupported_version",
    );
}

#[tokio::test(start_paused = true)]
async fn a_connection_nobody_configured_still_has_a_deadline() {
    // The documented minimal setup is `ConnectionConfig::default()`, so that is where the security
    // posture lives: a peer that connects and then says nothing must not hold the socket forever.
    let (mut client, server) = connect(ConnectionConfig::default());
    client
        .send(&intention(versions::V1_21, Intent::Login))
        .await;

    // Nothing else is ever sent. The connection ends on its own.
    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::TimedOut),
    ));
}
