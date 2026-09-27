//! Tests for the dialling half: connectors, the open hook, and a real socket.
//!
//! The rest of the suite runs both sides in process, which is what makes the other files fast and
//! deterministic. This one puts a [`Server`] and a [`Client`] on a real TCP socket, so that nothing
//! in between is a test double.

mod common;

use common::packets::*;
use common::*;
use passage_core::client::{Client, ClientError, Connected, connect_with};
use passage_core::router::Router;
use passage_core::server::Server;
use passage_core::versions;
use std::io;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// A server that answers a status request and closes.
fn status_server() -> Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|conn, _packet| {
            conn.state.push("status requested");
            conn.send(StatusResponse::text("mc.justchunks.net"))?;
            conn.close();
            Ok(())
        })
        .build()
}

/// A client that asks for the status and closes when it has it.
fn status_client() -> Router<Notes> {
    router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Status)?;
            conn.send(StatusRequest)?;
            Ok(())
        }))
        .note_and_close::<StatusResponse>()
        .build()
}

#[tokio::test]
async fn a_client_and_a_server_meet_over_a_real_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let addr = listener.local_addr().expect("bound");
    let shutdown = CancellationToken::new();
    let notes = Notes::default();

    let server = tokio::spawn(
        Server::new(listener)
            .state({
                let notes = notes.clone();
                move |peer: &SocketAddr| {
                    notes.push(format!("accepted {peer}"));
                    notes.clone()
                }
            })
            .dispatch(std::sync::Arc::new(status_server()))
            .graceful_shutdown(shutdown.clone())
            .serve(),
    );

    let outcome = Client::new(addr)
        .state(|_: &SocketAddr| Notes::default())
        .dispatch(std::sync::Arc::new(status_client()))
        .initial_version(versions::V26_1)
        .connect()
        .await
        .expect("the server is listening");

    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(
        outcome.state.lines(),
        [r#"StatusResponse { body: "mc.justchunks.net" }"#],
    );
    assert!(notes.saw("status requested"), "{:?}", notes.lines());

    shutdown.cancel();
    server.await.expect("the accept loop does not panic");
}

#[tokio::test]
async fn a_client_is_a_future_as_well_as_a_method() {
    // `IntoFuture`, so a client reads as one statement when nothing else has to happen first.
    let (server_io, client_io) = tokio::io::duplex(4096);
    let server = tokio::spawn(client(server_io, status_server()).connect());

    let outcome = client(client_io, status_client())
        .initial_version(versions::V26_1)
        .await
        .expect("preconnected");

    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let _ = server.await.expect("no panic");
}

#[tokio::test]
async fn a_client_with_no_open_hook_says_nothing_at_all() {
    // The dialling side speaks first, and a client that has nothing to say waits to be spoken to --
    // which, against a server that is also waiting, is exactly the deadlock the deadline bounds.
    let (server_io, client_io) = tokio::io::duplex(4096);
    let mut peer = RawClient::new(server_io);

    let outcome = client(client_io, router().build())
        .max_lifetime(Some(std::time::Duration::from_millis(50)))
        .connect()
        .await
        .expect("preconnected");

    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-timeout"),
    );
    peer.expect_eof().await;
}

#[tokio::test]
async fn a_dial_that_fails_is_reported_rather_than_run() {
    // Nothing listens on a port we bound and dropped. There is no connection, so there is no
    // outcome to report -- which is the whole reason this is a separate error type.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
    let addr = listener.local_addr().expect("bound");
    drop(listener);

    let error = Client::new(addr)
        .state(|_: &SocketAddr| Notes::default())
        .dispatch(std::sync::Arc::new(status_client()))
        .connect()
        .await
        .expect_err("nothing is listening");

    assert!(
        matches!(error, ClientError::Connect(ref err) if err.kind() == io::ErrorKind::ConnectionRefused),
        "{error}",
    );
}

#[tokio::test]
async fn a_layer_that_refuses_stops_the_client_before_the_protocol() {
    // The same stack as the server's, pointed the other way: whatever a client has to do to its own
    // socket before speaking -- a TLS handshake, a proxy negotiation -- is a layer, and one that
    // says no ends the attempt without a connection ever running.
    let (server_io, client_io) = tokio::io::duplex(4096);
    let mut peer = RawClient::new(server_io);

    let error = client(client_io, status_client())
        .layer(|_side: &Side| false)
        .connect()
        .await
        .expect_err("the layer refused");

    assert!(matches!(error, ClientError::Rejected), "{error}");
    peer.expect_eof().await;
}

#[tokio::test]
async fn a_connector_can_be_anything_that_produces_a_socket() {
    // `Connected` for a socket somebody else opened, and a closure for one the crate knows nothing
    // about. Between them, a client never needs a second entry point.
    let (server_io, client_io) = tokio::io::duplex(4096);
    let server = tokio::spawn(
        Client::new(Connected::new(server_io, Side::Server))
            .state(|_: &Side| Notes::default())
            .dispatch(std::sync::Arc::new(status_server()))
            .connect(),
    );

    let mut socket = Some(client_io);
    let outcome = Client::new(connect_with(move || {
        let io = socket.take().expect("dialled once");
        async move { Ok((io, Side::Client)) }
    }))
    .state(|_: &Side| Notes::default())
    .dispatch(std::sync::Arc::new(status_client()))
    .initial_version(versions::V26_1)
    .connect()
    .await
    .expect("the closure produced a socket");

    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(outcome.state.lines().len(), 1);
    let _ = server.await.expect("no panic");
}
