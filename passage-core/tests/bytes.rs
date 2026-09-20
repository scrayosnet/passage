//! What a connection does with bytes that are not a packet.
//!
//! A [`Scenario`](common::Scenario) cannot express any of this: the peer here is not a connection
//! but a socket, sending an ID nobody registered, a length prefix that lies, or nothing at all.

mod common;

use common::packets::*;
use common::*;
use passage_core::wire::Writer;
use passage_core::{Packet, ProtocolVersion, versions};
use std::time::Duration;

/// A server that answers a status request and nothing else.
fn server() -> passage_core::router::Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|conn, _packet| {
            conn.send(StatusResponse::text("mc.justchunks.net"))?;
            Ok(())
        })
        .build()
}

/// Walks a raw peer up to the status phase.
async fn reach_status(peer: &mut RawClient, version: ProtocolVersion) {
    peer.send(&Handshake::new(version, Intent::Status)).await;
    peer.version = version;
}

#[tokio::test]
async fn the_id_a_server_writes_is_the_one_a_peer_reads() {
    // The whole framing contract in one exchange: length, then ID, then payload -- decoded by
    // something that shares no code with the connection that wrote it.
    let (mut peer, server) = Served::new(server()).start();

    reach_status(&mut peer, versions::V26_1).await;
    peer.send(&StatusRequest).await;

    let response = peer.expect::<StatusResponse>().await;
    assert_eq!(response.body, "mc.justchunks.net");

    drop(peer);
    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-closed"),
    );
}

#[tokio::test]
async fn an_id_nobody_registered_is_the_peers_doing() {
    let (mut peer, server) = Served::new(server()).start();

    reach_status(&mut peer, versions::V26_1).await;
    // 0x7F exists in no phase of the test protocol.
    peer.send_raw(0x7F, &[]).await;

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "unknown_packet");
    assert!(error.is_peer_error());
}

#[tokio::test]
async fn a_payload_that_does_not_match_its_packet_is_the_peers_doing() {
    // The ID routes, and then the packet's own decoder disagrees. Either we are misreading it or
    // the peer is smuggling data past us, and both are worth failing on.
    let (mut peer, server) = Served::new(server()).start();

    reach_status(&mut peer, versions::V26_1).await;
    peer.send_raw(
        StatusRequest::id(versions::V26_1).expect("exists"),
        b"extra",
    )
    .await;

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "malformed_packet");
    assert!(error.is_peer_error());
    assert!(error.to_string().contains("StatusRequest"), "{error}");
}

#[tokio::test]
async fn a_hostile_length_prefix_is_a_peer_error_not_a_panic() {
    // A handshake whose `server_address` claims a length of -1. Decoded as a `usize` that is
    // 18446744073709551615, which is what used to reach `vec![0; len]`.
    let (mut peer, server) = Served::new(server()).start();

    let mut payload = bytes::BytesMut::new();
    let mut writer = Writer::new(&mut payload);
    writer.var_int(767); // the protocol version
    writer.raw(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]); // the string length: -1
    peer.send_raw(0x00, &payload).await;

    let outcome = server.await.expect("the connection task must not panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "malformed_packet");
    assert!(
        error.is_peer_error(),
        "a payload the peer sent is the peer's doing, not ours",
    );
    // The field the decoder named survives into the one line an operator reads.
    assert!(error.to_string().contains("server_address"), "{error}");
}

#[tokio::test]
async fn an_oversized_frame_is_rejected_before_it_is_buffered() {
    let (mut peer, server) = Served::new(server())
        .config(|config| config.wire_options.max_frame_len = 128)
        .start();

    // Announce a 1 MiB frame in three bytes, then send nothing.
    let mut announcement = bytes::BytesMut::new();
    Writer::new(&mut announcement).var_int(1024 * 1024);
    peer.send_bytes(&announcement).await;

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "codec");
    assert!(error.to_string().contains("128"), "{error}");
}

#[tokio::test]
async fn a_frame_that_arrives_in_pieces_is_backpressure_not_an_error() {
    // A peer is free to write a packet one byte at a time, and a connection that treated a short
    // read as a failure would drop every client on a slow link.
    let (mut peer, server) = Served::new(server()).start();

    let mut frame = bytes::BytesMut::new();
    {
        let mut payload = bytes::BytesMut::new();
        let mut writer = Writer::new(&mut payload);
        writer.var_int(0x00);
        Handshake::new(versions::V26_1, Intent::Status)
            .encode(&mut writer, versions::V26_1)
            .expect("encodes");
        let mut header = Writer::new(&mut frame);
        header.length("packet_length", payload.len()).expect("fits");
        header.raw(&payload);
    }

    for byte in frame.iter() {
        peer.send_bytes(&[*byte]).await;
        tokio::task::yield_now().await;
    }

    // The handshake arrived, so the status phase is reachable.
    peer.version = versions::V26_1;
    peer.send(&StatusRequest).await;
    assert_eq!(
        peer.expect::<StatusResponse>().await.body,
        "mc.justchunks.net",
    );

    drop(peer);
    let _ = server.await.expect("no panic");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_stops_reading_does_not_outlast_its_deadline() {
    // The cheapest attack on a protocol server: connect, ask for something big, never read the
    // answer. The write buffer fills, the write stops making progress, and if that happened
    // anywhere the loop could not see, the connection would sit there past every deadline and past
    // any shutdown.
    let big = router()
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|conn, _packet| {
            conn.send(StatusResponse::text(&"x".repeat(4000)))?;
            Ok(())
        })
        .build();
    let (mut peer, server) = Served::new(big)
        .buffer(16)
        .config(|config| config.max_lifetime = Some(Duration::from_secs(5)))
        .start();

    reach_status(&mut peer, versions::V26_1).await;
    peer.send(&StatusRequest).await;

    // Nothing ever reads `peer`.
    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-timeout"),
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_says_nothing_at_all_is_still_bounded() {
    let (_peer, server) = Served::new(server())
        .config(|config| config.max_lifetime = Some(Duration::from_secs(30)))
        .start();

    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-timeout"),
    );
}

#[tokio::test]
async fn a_peer_that_vanishes_is_noticed_rather_than_waited_for() {
    let (peer, server) = Served::new(server()).start();
    drop(peer);

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("an ending");
    assert_eq!(error.reason(), "peer-closed");
    assert!(!error.can_reply(), "there is nobody left to read an answer");
}
