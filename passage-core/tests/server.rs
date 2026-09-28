//! Tests for the accept loop: layers, per-connection state, shutdown, draining, accept errors.
//!
//! The listener is a channel rather than a socket, so the tests decide exactly when a connection
//! arrives and never touch the network. That a [`Listener`](passage_core::server::Listener) can be
//! implemented over a channel at all is part of what is being tested.

mod common;

use bytes::Bytes;
use common::packets::*;
use common::*;
use futures::future::BoxFuture;
use passage_core::connection::{ConnRef, DispatchError, Dispatcher, make_with};
use passage_core::router::{Layer, Router};
use passage_core::server::Server;
use passage_core::versions;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, BufReader, DuplexStream};
use tracing::Level;

/// A server that answers a status request with the address the state factory recorded.
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

/// Runs a status client over `io` and returns what it saw.
async fn ping(io: DuplexStream) -> Vec<String> {
    client(io, status_client())
        .initial_version(versions::V26_3)
        .connect()
        .await
        .expect("preconnected")
        .state
        .lines()
}

#[tokio::test]
async fn a_connection_the_listener_accepts_is_served() {
    let running = Harness::new(status_server()).start();
    let saw = ping(running.incoming.connect(1)).await;

    assert_eq!(saw, [r#"StatusResponse { body: "mc.justchunks.net" }"#]);
    assert!(
        !running.task.is_finished(),
        "one connection is not the end of it"
    );
}

#[tokio::test]
async fn the_state_factory_sees_the_address_the_listener_reported() {
    // The point of a factory rather than a value: the address is only known per connection.
    let running = Harness::new(status_server()).start();
    ping(running.incoming.connect(25_565)).await;

    assert_eq!(running.notes.find("accepted"), "accepted 127.0.0.1:25565");
}

#[tokio::test]
async fn every_connection_gets_its_own_state_and_the_server_outlives_all_of_them() {
    let running = Harness::new(status_server()).start();

    // Interleaved on purpose: two connections are live at once.
    let first = ping(running.incoming.connect(1));
    let second = ping(running.incoming.connect(2));
    let (first, second) = tokio::join!(first, second);

    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(running.notes.count_lines("accepted"), 2);
    assert!(!running.task.is_finished());
}

#[tokio::test]
async fn an_accept_error_does_not_stop_the_server() {
    // A peer that hung up between the SYN and our accept. The next connection must still be served,
    // which is the whole claim.
    let running = Harness::new(status_server()).start();
    running.incoming.fail(io::ErrorKind::ConnectionAborted);

    let saw = ping(running.incoming.connect(1)).await;
    assert_eq!(saw.len(), 1);
}

/// A layer in the shape of a PROXY header reader: it rewrites the address everything downstream
/// sees, and refuses a peer it will not vouch for.
struct RewriteAddr {
    refuse_port: u16,
}

impl<Io> Layer<Io, Peer> for RewriteAddr
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, peer: Peer) -> Option<(Io, Peer)> {
        if peer.port() == self.refuse_port {
            // A real one would count this, and say whether it was a malformed header or an
            // untrusted source. Nothing downstream is told, and nothing downstream needs to be.
            return None;
        }
        let port = peer.port() + 1_000;
        Some((io, peer.at(port)))
    }
}

/// A layer that changes the socket *type*, which is what a TLS layer does and the reason
/// [`Layer::Io`] exists. `BufReader` stands in for the upgrade; the point is that what the
/// connection ends up running on is not what the listener produced.
struct Upgrade;

impl<Io> Layer<Io, Peer> for Upgrade
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = BufReader<Io>;

    async fn admit(&self, io: Io, peer: Peer) -> Option<(BufReader<Io>, Peer)> {
        Some((BufReader::new(io), peer))
    }
}

/// A rate limiter that keeps its own books, which is the whole shape being tested: it is the only
/// thing that knows it refused someone and why, so it is the thing that counts it.
struct RateLimiter {
    blocked_port: u16,
    admitted: Arc<AtomicUsize>,
    refused: Arc<AtomicUsize>,
}

impl<Io> Layer<Io, Peer> for RateLimiter
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, peer: Peer) -> Option<(Io, Peer)> {
        let allowed = peer.port() != self.blocked_port;
        // Counted here, at the decision, where the reason is still in hand.
        let counter = if allowed {
            &self.admitted
        } else {
            &self.refused
        };
        counter.fetch_add(1, Ordering::Relaxed);
        allowed.then_some((io, peer))
    }
}

#[tokio::test]
async fn a_refused_peer_never_reaches_the_protocol_and_the_limiter_counts_its_own() {
    // Rate limiting as a layer, in the only place it can see the address a PROXY layer reported and
    // still not hold up the accept loop. `&self` is what lets it be a named type with state, so
    // "how many did we turn away" needs no reporting hook at all -- and the driver, which has
    // nothing useful to say about somebody else's policy, says nothing.
    let (logs, _guard) = record_logs();
    let admitted = Arc::new(AtomicUsize::new(0));
    let refused = Arc::new(AtomicUsize::new(0));

    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let notes = harness.notes;
    let task = tokio::spawn(
        harness
            .server
            .layer(RateLimiter {
                blocked_port: 2,
                admitted: Arc::clone(&admitted),
                refused: Arc::clone(&refused),
            })
            .serve(),
    );

    let mut rejected = RawClient::new(incoming.connect(2));
    rejected.expect_eof().await;
    assert_eq!(ping(incoming.connect(1)).await.len(), 1);

    assert_eq!(refused.load(Ordering::Relaxed), 1);
    assert_eq!(admitted.load(Ordering::Relaxed), 1);
    // The refusal never reached the state factory, and the server invented nothing to say about
    // somebody else's policy: there is no ending on the record for a connection that never ran.
    assert_eq!(notes.count_lines("accepted"), 1);
    assert_eq!(logs.count("connection failed"), 0);

    task.abort();
}

#[tokio::test]
async fn layers_run_in_the_order_they_are_written() {
    // Only one order can produce this result, which is the point of asserting it this way: the
    // rewrite turns port 2 into 1002, and the closure after it refuses 1002. Written the other way
    // round the closure would see port 2, admit it, and the connection would run.
    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let task = tokio::spawn(
        harness
            .server
            .layer(RewriteAddr { refuse_port: 0 })
            .layer(|peer: &Peer| peer.port() != 1_002)
            .serve(),
    );

    let mut refused = RawClient::new(incoming.connect(2));
    refused.expect_eof().await;

    // And a peer the closure does not object to is served, so the refusal above was the closure's
    // doing and not the stack failing outright.
    assert_eq!(ping(incoming.connect(3)).await.len(), 1);
    task.abort();
}

#[tokio::test]
async fn a_layer_can_change_the_socket_type_and_the_address_downstream_sees() {
    // Between them, everything a preamble is for. The state factory is handed what the *last* layer
    // left, not what the listener first reported, which is the whole point of the PROXY case.
    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let notes = harness.notes;
    let task = tokio::spawn(
        harness
            .server
            .layer(RewriteAddr { refuse_port: 0 })
            .layer(Upgrade)
            .serve(),
    );

    assert_eq!(ping(incoming.connect(25_565)).await.len(), 1);
    assert_eq!(notes.find("accepted"), "accepted 127.0.0.1:26565");
    task.abort();
}

#[tokio::test]
async fn a_refused_peer_costs_the_next_one_nothing() {
    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let task = tokio::spawn(harness.server.layer(RewriteAddr { refuse_port: 2 }).serve());

    let mut refused = RawClient::new(incoming.connect(2));
    refused.expect_eof().await;
    assert_eq!(ping(incoming.connect(1)).await.len(), 1);

    task.abort();
}

/// A dispatcher that panics on the first frame, standing in for a handler bug.
struct Panicking;

impl Dispatcher<Notes> for Panicking {
    fn on_frame<'a>(
        &self,
        _conn: ConnRef<'a, Notes>,
        _id: i32,
        _payload: Bytes,
    ) -> BoxFuture<'a, Result<(), DispatchError>> {
        panic!("a handler bug");
    }
}

#[tokio::test]
async fn a_panicking_connection_is_reported_loudly_rather_than_vanishing() {
    // The one outcome that could otherwise escape reporting entirely: the connection task unwinds,
    // the join handle is dropped, and nothing downstream ever hears about it. It is the driver's
    // own record that closes that hole, so the driver's own record is what this reads -- and at
    // `error`, because a handler panicking is ours, not the peer's.
    let (logs, _guard) = record_logs();
    let (listener, incoming) = {
        let harness = Harness::new(router().build());
        (harness.server, harness.incoming)
    };
    // The router is replaced wholesale: what is under test is a dispatcher that panics.
    let task = tokio::spawn(listener.dispatch(make_with(|| Panicking)).serve());

    let mut peer = RawClient::new(incoming.connect(1));
    peer.send_raw(0x00, &[]).await;
    peer.expect_eof().await;

    let (level, text) = logs.find("connection panicked");
    assert_eq!(level, Level::ERROR, "{text}");
    assert!(text.contains("a handler bug"), "{text}");

    // And the server carries on: one connection's bug is not the listener's problem.
    assert!(!task.is_finished());
    task.abort();
}

#[tokio::test]
async fn an_ordinary_ending_is_reported_quietly() {
    // A scanner that takes its status and leaves looks exactly like a hangup, and neither may ever
    // wake anybody up. The level is the whole assertion.
    let (logs, _guard) = record_logs();
    let running = Harness::new(status_server()).start();
    ping(running.incoming.connect(1)).await;

    let closed = logs.all("connection closed");
    assert_eq!(closed.len(), 2, "both sides closed: {closed:#?}");
    for (level, text) in closed {
        assert_eq!(level, Level::DEBUG, "{text}");
    }
    assert_eq!(logs.count("connection failed"), 0);
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_accepting_and_ends_the_connections_it_already_has() {
    // Cancelling the server cancels its connections, and `drain_timeout` bounds how long the
    // server waits for all of them.
    let running = Harness::new(status_server()).start();

    // A connection that is live and has been answered, but not closed.
    let mut live = RawClient::new(running.incoming.connect(1)).at(versions::V26_3);
    live.send(&Handshake::new(versions::V26_3, Intent::Status))
        .await;
    tokio::task::yield_now().await;

    // A connection handed over after the signal is never served at all.
    let mut late = RawClient::new(running.incoming.connect(2)).at(versions::V26_3);
    running.shutdown.cancel();

    live.expect_eof().await;
    late.expect_eof().await;
    tokio::time::timeout(Duration::from_secs(60), running.task)
        .await
        .expect("the drain ends")
        .expect("the accept loop does not panic");
}

/// Accepts the handshake and then never finishes, so the connection cannot end on its own.
///
/// A named `async fn` rather than a closure: a closure whose future captures the connection cannot
/// be inferred as higher-ranked over its lifetime.
async fn never_finishes(conn: ConnRef<'_, Notes>, packet: Handshake) -> Result<(), DispatchError> {
    conn.with(|c| accept(c, &packet))?;
    std::future::pending::<()>().await;
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn the_drain_timeout_bounds_how_long_the_server_waits() {
    // A connection that will not end, and a server that has to come back anyway.
    let slow = router().handle_async(never_finishes).build();

    let harness = Harness::new(slow);
    let incoming = harness.incoming;
    let shutdown = harness.shutdown.clone();
    let task = tokio::spawn(
        harness
            .server
            .drain_timeout(Some(Duration::from_secs(10)))
            .max_lifetime(None)
            .serve(),
    );

    let mut peer = RawClient::new(incoming.connect(1)).at(versions::V26_3);
    peer.send(&Handshake::new(versions::V26_3, Intent::Status))
        .await;
    tokio::task::yield_now().await;

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(60), task)
        .await
        .expect("the drain timeout ends the wait")
        .expect("the accept loop does not panic");
}

#[tokio::test]
async fn a_connection_limit_is_a_limit_on_what_is_accepted() {
    // The permit is taken before the accept, so a server at its limit stops taking sockets off the
    // listener rather than accepting them and starving them afterwards.
    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let notes = harness.notes;
    let task = tokio::spawn(harness.server.max_connections(Some(1)).serve());

    // The first one is accepted and answered; only then is the permit free again.
    assert_eq!(ping(incoming.connect(1)).await.len(), 1);
    assert_eq!(ping(incoming.connect(2)).await.len(), 1);
    assert_eq!(notes.count_lines("accepted"), 2);

    task.abort();
}

#[tokio::test]
async fn a_server_is_a_future_as_well_as_a_method() {
    // `IntoFuture` is what lets a server be awaited directly rather than through `serve`, which is
    // how the crate documents it.
    let harness = Harness::new(status_server());
    let incoming = harness.incoming;
    let shutdown = harness.shutdown.clone();
    let server: Server<_, _, _, _> = harness.server;
    let task = tokio::spawn(async move { server.await });

    assert_eq!(ping(incoming.connect(1)).await.len(), 1);
    shutdown.cancel();
    task.await.expect("the accept loop does not panic");
}
