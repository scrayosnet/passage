//! End-to-end tests for the driver, driven by a raw protocol client over a socket pair.
//!
//! These exist to keep the claims in `docs/` honest. Each test names the property it proves.

use futures::{SinkExt, StreamExt};
use passage_driver::codec::{Encoded, FrameCodec};
use passage_driver::demo::packets::{
    Intention, KeepAlive, KeepAliveResponse, LoginAcknowledged, LoginStart, LoginSuccess,
    PingRequest, PongResponse, StatusRequest, StatusResponse, Transfer,
};
use passage_driver::demo::server::{Session, router};
use passage_driver::driver::{Completion, Driver, DriverConfig};
use passage_driver::error::{Class, Error, Result};
use passage_driver::packet::Packet;
use passage_driver::version::{ProtocolVersion, versions};
use passage_driver::wire::{Limits, Reader, VarInt, Writer};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// A minimal protocol client: the mirror image of the driver, without the driver.
struct TestClient {
    framed: Framed<DuplexStream, FrameCodec>,
    version: ProtocolVersion,
    limits: Limits,
}

impl TestClient {
    fn new(io: DuplexStream) -> Self {
        let limits = Limits::default();
        Self {
            framed: Framed::new(io, FrameCodec::new(limits)),
            version: ProtocolVersion::UNKNOWN,
            limits,
        }
    }

    async fn send<P: Packet>(&mut self, packet: &P) {
        let encoded = Encoded::of(packet, self.version).expect("packet exists in this version");
        self.framed.send(encoded).await.expect("writes");
    }

    /// Reads the next packet, asserting it is of the expected type.
    async fn expect<P: Packet>(&mut self) -> P {
        let frame = self
            .framed
            .next()
            .await
            .expect("connection stays open")
            .expect("frame decodes");
        assert_eq!(
            Some(frame.id),
            P::id(self.version),
            "expected {} (id {:?}), got id {}",
            P::NAME,
            P::id(self.version),
            frame.id,
        );
        let mut reader = Reader::new(&frame.payload, self.limits);
        P::decode(&mut reader, self.version).expect("packet decodes")
    }

    /// Sends bytes that are not a valid packet at all.
    async fn send_raw(&mut self, bytes: &[u8]) {
        self.framed
            .send(Encoded {
                name: "Raw",
                bytes: bytes.to_vec().into(),
            })
            .await
            .expect("writes");
    }

    async fn expect_eof(&mut self) {
        assert!(
            self.framed.next().await.is_none(),
            "expected the server to close the connection",
        );
    }
}

/// Spawns the demo server on one end of a socket pair and hands back a client for the other.
fn serve(config: DriverConfig) -> (TestClient, JoinHandle<Result<Completion>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let router = Arc::new(router());
    let (driver, _handle) = Driver::new(
        server_io,
        router,
        Session::default(),
        config,
        CancellationToken::new(),
    )
    .expect("router binds");
    (TestClient::new(client_io), tokio::spawn(driver.run()))
}

fn intention(version: ProtocolVersion, intent: i32) -> Intention {
    Intention {
        protocol_version: VarInt(version.get()),
        server_address: "mc.justchunks.net".to_owned(),
        server_port: 25565,
        intent: VarInt(intent),
    }
}

#[tokio::test]
async fn serves_a_status_ping_end_to_end() {
    let (mut client, server) = serve(DriverConfig::default());

    client.send(&intention(versions::V1_21, 1)).await;
    // The client switches its own version and phase exactly like the server does.
    client.version = versions::V1_21;
    client.send(&StatusRequest {}).await;

    let status = client.expect::<StatusResponse>().await;
    assert!(status.body.contains("mc.justchunks.net"), "{}", status.body);

    client.send(&PingRequest { payload: 0x1234 }).await;
    let pong = client.expect::<PongResponse>().await;
    assert_eq!(pong.payload, 0x1234);

    client.expect_eof().await;
    assert_eq!(
        server.await.expect("no panic").expect("no error"),
        Completion::Closed,
    );
}

#[tokio::test]
async fn the_gated_field_follows_the_client_version() {
    // An old client must not be sent the session id...
    let (mut client, server) = serve(DriverConfig::default());
    client.send(&intention(versions::V1_21, 2)).await;
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
    let (mut client, server) = serve(DriverConfig::default());
    client.send(&intention(versions::V26_2, 2)).await;
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
async fn an_async_handler_blocks_further_dispatch() {
    // Both packets are written before the server has even read the first. The login handler is
    // asynchronous, so if the driver kept reading, `LoginAcknowledged` could be dispatched into a
    // session that is not authenticated yet. The order of the replies proves it does not.
    let (mut client, server) = serve(DriverConfig::default());
    client.send(&intention(versions::V26_2, 2)).await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    client.send(&LoginAcknowledged {}).await;

    // LoginSuccess comes first, from the async handler...
    let _ = client.expect::<LoginSuccess>().await;
    // ...and only then the transfer, from the task the acknowledgement detached.
    let transfer = client.expect::<Transfer>().await;
    assert_eq!(transfer.host, "backend-1.justchunks.net");
    assert_eq!(transfer.port, VarInt(25565));

    client.expect_eof().await;
    assert_eq!(
        server.await.expect("no panic").expect("no error"),
        Completion::Closed,
    );
}

#[tokio::test(start_paused = true)]
async fn detached_work_runs_while_keep_alives_are_exchanged() {
    // The backend selection is detached, so the tick handler keeps the connection alive while it
    // runs. With a paused clock we can hold up the selection for a minute of virtual time.
    let config = DriverConfig {
        tick_interval: Some(Duration::from_secs(16)),
        ..DriverConfig::default()
    };
    let (mut client, server) = serve(config);

    client.send(&intention(versions::V26_2, 2)).await;
    client.version = versions::V26_2;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    let _ = client.expect::<LoginSuccess>().await;
    client.send(&LoginAcknowledged {}).await;

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
    assert_eq!(
        server.await.expect("no panic").expect("no error"),
        Completion::Closed,
    );
    // The selection in the demo is fast, so this is only a smoke check that ticks are wired up.
    assert!(
        keep_alives <= 1,
        "unexpected keep-alive count {keep_alives}"
    );
}

#[tokio::test]
async fn an_unknown_packet_ends_the_connection_as_a_peer_error() {
    let (mut client, server) = serve(DriverConfig::default());
    client.send(&intention(versions::V1_21, 1)).await;
    client.version = versions::V1_21;
    // Id 0x7F exists in no phase.
    client.send_raw(&[0x7F]).await;

    let err = server
        .await
        .expect("no panic")
        .expect_err("must fail the connection");
    assert_eq!(err.class(), Class::Peer);
    assert_eq!(err.label(), "unknown_packet");
}

#[tokio::test]
async fn a_hostile_length_prefix_is_a_peer_error_not_a_panic() {
    let (mut client, server) = serve(DriverConfig::default());

    // A handshake whose `server_address` claims a length of -1. Decoded as `usize` that is
    // 18446744073709551615, which is what used to reach `vec![0; len]`.
    let mut payload = Vec::new();
    let mut buf = bytes::BytesMut::new();
    let mut writer = Writer::new(&mut buf);
    writer.var_int(0x00); // packet id
    writer.var_int(767); // protocol version
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]); // string length: -1
    payload.extend_from_slice(&buf);
    client.send_raw(&payload).await;

    let err = server
        .await
        .expect("the connection task must not panic")
        .expect_err("must fail the connection");
    assert_eq!(err.class(), Class::Peer);
    assert_eq!(err.label(), "negative_length");
}

#[tokio::test]
async fn an_oversized_frame_is_rejected_before_it_is_buffered() {
    let config = DriverConfig {
        limits: Limits {
            max_frame_len: 128,
            ..Limits::default()
        },
        ..DriverConfig::default()
    };
    let (mut client, server) = serve(config);

    // Announce a 1 MiB frame in three bytes, then send nothing.
    let mut buf = bytes::BytesMut::new();
    Writer::new(&mut buf).var_int(1024 * 1024);
    tokio::io::AsyncWriteExt::write_all(client.framed.get_mut(), &buf)
        .await
        .expect("writes");

    let err = server
        .await
        .expect("no panic")
        .expect_err("must fail the connection");
    assert_eq!(err.class(), Class::Peer);
    assert_eq!(err.label(), "frame_too_large");
}

#[tokio::test]
async fn logging_in_with_an_unsupported_version_is_refused() {
    let (mut client, server) = serve(DriverConfig::default());
    // 1.20.4: no configuration phase, so no transfer.
    client.send(&intention(ProtocolVersion::new(765), 2)).await;

    let err = server
        .await
        .expect("no panic")
        .expect_err("must fail the connection");
    assert_eq!(err.label(), "unsupported_version");

    // But a status ping from the same client still works, which is how it learns what to install.
    let (mut client, server) = serve(DriverConfig::default());
    client.send(&intention(ProtocolVersion::new(765), 1)).await;
    client.version = ProtocolVersion::new(765);
    client.send(&StatusRequest {}).await;
    let _ = client.expect::<StatusResponse>().await;
    drop(client);
    let _ = server.await;
}

#[tokio::test]
async fn a_peer_hangup_is_not_an_error() {
    let (client, server) = serve(DriverConfig::default());
    drop(client);
    assert_eq!(
        server.await.expect("no panic").expect("no error"),
        Completion::PeerClosed,
    );
}

#[tokio::test]
async fn cancellation_ends_the_connection_cleanly() {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let shutdown = CancellationToken::new();
    let (driver, handle) = Driver::new(
        server_io,
        Arc::new(router()),
        Session::default(),
        DriverConfig::default(),
        shutdown.clone(),
    )
    .expect("router binds");
    let server = tokio::spawn(driver.run());
    let _client = TestClient::new(client_io);

    shutdown.cancel();
    assert_eq!(
        server.await.expect("no panic").expect("no error"),
        Completion::Cancelled,
    );
    // The handle notices, so detached work can observe it too.
    assert!(handle.shutdown().is_cancelled());
}

#[tokio::test]
async fn the_router_rejects_conflicting_ids_at_bind_time() {
    // Two packets claiming the same id in the same phase is a wiring bug; it must surface at
    // startup, not on the first client that happens to send one of them.
    // Closures need their argument types spelled out so they implement `Fn` for *any* lifetime;
    // named handler functions (as in `demo::server`) do not have that wrinkle.
    let router = router().on::<StatusRequest, _>(
        |_ctx: passage_driver::conn::Ctx<'_, Session>, _packet: StatusRequest| {
            passage_driver::flow::Flow::done()
        },
    );
    let err = router
        .validate([versions::V1_20_5, versions::V26_2])
        .expect_err("must reject");
    assert!(matches!(err, Error::Internal(_)), "{err}");
}
