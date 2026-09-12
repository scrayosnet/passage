//! Tests for the accept loop: layers, per-connection state, shutdown, draining, accept errors.
//!
//! The listener is a channel rather than a socket, so the tests decide exactly when a connection
//! arrives and never touch the network. That [`Listener`] can be implemented over a socket pair at
//! all is part of what is being tested.

mod common;

use common::{TestClient, intention, intention_to, record_logs};
use passage_driver::conn::{ConnectionConfig, Ctx};
use passage_driver::demo::packets::{
    Intent, Intention, PingRequest, PongResponse, StatusRequest, StatusResponse,
};
use passage_driver::demo::server::{Session, router};
use passage_driver::error::Result;
use passage_driver::packet::Phase;
use passage_driver::router::Router;
use passage_driver::server::{Layer, Listener, Server, make_with, serve};
use passage_driver::version::versions;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, BufReader, DuplexStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Level;

/// What the test hands to the accept loop.
type Accepted = io::Result<(DuplexStream, SocketAddr)>;

/// A listener fed by a channel: the test decides what is accepted, and when.
///
/// Three lines of trait, which is the point: a listener is a *source* of sockets and nothing else.
struct TestListener {
    incoming: mpsc::UnboundedReceiver<Accepted>,
}

impl Listener for TestListener {
    type Io = DuplexStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> Accepted {
        match self.incoming.recv().await {
            Some(accepted) => accepted,
            // Nothing more is coming. A quiet listener blocks; it does not report an error in a
            // loop, and the accept loop must not treat "quiet" as anything at all.
            None => std::future::pending().await,
        }
    }
}

/// A layer in the shape of a PROXY header reader: it rewrites the address everything downstream
/// sees, and refuses a peer it will not vouch for.
struct RewriteAddr {
    refuse_port: u16,
}

impl<Io> Layer<Io, SocketAddr> for RewriteAddr
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, mut addr: SocketAddr) -> Option<(Io, SocketAddr)> {
        if addr.port() == self.refuse_port {
            // A real one would count this, and say whether it was a malformed header or an
            // untrusted source. Nothing downstream is told, and nothing downstream needs to be.
            return None;
        }
        addr.set_port(addr.port() + 1_000);
        Some((io, addr))
    }
}

/// A layer that changes the socket *type*, which is what a TLS layer does and the reason
/// [`Layer::Io`] exists. `BufReader` stands in for the upgrade; the point is that what the
/// connection ends up running on is not what the listener produced.
struct Upgrade;

impl<Io> Layer<Io, SocketAddr> for Upgrade
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = BufReader<Io>;

    async fn admit(&self, io: Io, addr: SocketAddr) -> Option<(BufReader<Io>, SocketAddr)> {
        Some((BufReader::new(io), addr))
    }
}

/// What a layer publishes, which is the only thing anyone else needs from it.
#[derive(Default)]
struct Counts {
    admitted: AtomicUsize,
    refused: AtomicUsize,
}

/// A rate limiter that keeps its own books, which is the whole shape being tested: it is the only
/// thing that knows it refused someone and why, so it is the thing that counts it. Nothing
/// downstream is told, and nothing downstream has to grow a vocabulary for it.
struct RateLimiter {
    blocked_port: u16,
    counts: Arc<Counts>,
}

impl<Io> Layer<Io, SocketAddr> for RateLimiter
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, addr: SocketAddr) -> Option<(Io, SocketAddr)> {
        let allowed = addr.port() != self.blocked_port;
        // Counted here, at the decision, where the reason is still in hand.
        let counter = if allowed {
            &self.counts.admitted
        } else {
            &self.counts.refused
        };
        counter.fetch_add(1, Ordering::Relaxed);
        allowed.then_some((io, addr))
    }
}

/// The other end of a [`TestListener`].
struct Incoming(mpsc::UnboundedSender<Accepted>);

impl Incoming {
    /// Presents a connection from `port`, and returns the client side of it.
    fn connect(&self, port: u16) -> TestClient {
        let (server_io, client_io) = tokio::io::duplex(4096);
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        self.0
            .send(Ok((server_io, addr)))
            .expect("the accept loop is running");
        TestClient::new(client_io)
    }

    /// Presents a failed accept.
    fn fail(&self, kind: io::ErrorKind) {
        self.0
            .send(Err(io::Error::new(kind, "as if the peer had gone away")))
            .expect("the accept loop is running");
    }
}

/// Everything a test needs to talk to a running server.
struct Harness {
    incoming: Incoming,
    shutdown: CancellationToken,
    server: JoinHandle<()>,
}

/// Starts a server on a channel-backed listener.
fn start(router: Router<Session>, drain: Option<Duration>) -> Harness {
    let (tx, rx) = mpsc::unbounded_channel();
    let shutdown = CancellationToken::new();

    let mut server = serve(
        TestListener { incoming: rx },
        Arc::new(router),
        // The point of a factory rather than a value: the address is only known per connection.
        |addr| Session {
            peer: Some(*addr),
            ..Session::default()
        },
    )
    .config(ConnectionConfig::default())
    .graceful_shutdown(shutdown.clone());

    if let Some(after) = drain {
        server = server.drain_timeout(after);
    }

    Harness {
        incoming: Incoming(tx),
        shutdown,
        server: tokio::spawn(server.run()),
    }
}

fn demo(drain: Option<Duration>) -> Harness {
    start(router().expect("the demo router is well-formed"), drain)
}

/// A status ping: the shortest complete conversation the demo server has.
async fn ping(client: &mut TestClient, host: &str) -> StatusResponse {
    client.version = versions::V1_21;
    client
        .send(&intention_to(host, versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;
    let response = client.expect::<StatusResponse>().await;
    client.send(&PingRequest { payload: 7 }).await;
    assert_eq!(client.expect::<PongResponse>().await.payload, 7);
    client.expect_eof().await;
    response
}

#[tokio::test]
async fn serves_a_connection_the_listener_accepts() {
    let harness = demo(None);
    let mut client = harness.incoming.connect(1);

    let response = ping(&mut client, "mc.justchunks.net").await;
    assert!(response.body.contains("mc.justchunks.net"), "{response:?}");
    assert!(!harness.server.is_finished());
}

#[tokio::test]
async fn every_connection_gets_its_own_state() {
    let harness = demo(None);
    let mut first = harness.incoming.connect(1);
    let mut second = harness.incoming.connect(2);

    // Interleaved on purpose: were the state shared, the second handshake would overwrite the host
    // the first one recorded before the first status response is built.
    first.version = versions::V1_21;
    second.version = versions::V1_21;
    first
        .send(&intention_to(
            "first.example",
            versions::V1_21,
            Intent::Status,
        ))
        .await;
    second
        .send(&intention_to(
            "second.example",
            versions::V1_21,
            Intent::Status,
        ))
        .await;
    first.send(&StatusRequest).await;
    second.send(&StatusRequest).await;

    let first_body = first.expect::<StatusResponse>().await.body;
    let second_body = second.expect::<StatusResponse>().await.body;
    assert!(first_body.contains("first.example"), "{first_body}");
    assert!(second_body.contains("second.example"), "{second_body}");
}

/// A router whose status response is the peer address the state factory was given, and nothing
/// else -- so an assertion about the address is an assertion about the state alone.
fn peer_echo_router() -> Router<Session> {
    Router::builder()
        .on::<Intention>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Status)
        })
        .on::<StatusRequest>(|ctx: Ctx<'_, Session>, _: StatusRequest| {
            ctx.send(StatusResponse {
                body: format!("{:?}", ctx.state.peer),
            })?;
            ctx.close()
        })
        .build()
        .expect("builds")
}

#[tokio::test]
async fn the_state_factory_sees_the_address_the_listener_reported() {
    let harness = start(peer_echo_router(), None);
    let mut client = harness.incoming.connect(25_565);

    client.version = versions::V1_21;
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;

    let body = client.expect::<StatusResponse>().await.body;
    assert!(body.contains("127.0.0.1:25565"), "{body}");
}

#[tokio::test]
async fn a_server_can_be_assembled_by_name() {
    // The builder path, with the connection knobs forwarded onto it rather than reached through a
    // `ConnectionConfig` literal. `.run()` exists here only because a listener, something to
    // dispatch to and a state factory have all been set -- each one is a type parameter that starts
    // unset, so forgetting one is a missing method rather than a server that does nothing.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        Server::builder()
            .listener(TestListener { incoming: rx })
            .dispatch(Arc::new(router().expect("builds")))
            .state(|addr| Session {
                peer: Some(*addr),
                ..Session::default()
            })
            .tick_interval(Duration::from_secs(16))
            .max_lifetime(Some(Duration::from_secs(30)))
            .max_connections(4)
            .run(),
    );

    let incoming = Incoming(tx);
    let mut client = incoming.connect(1);
    let response = ping(&mut client, "mc.justchunks.net").await;
    assert!(response.body.contains("mc.justchunks.net"), "{response:?}");

    server.abort();
}

#[tokio::test]
async fn an_accept_error_does_not_stop_the_server() {
    let harness = demo(None);

    // A peer that hung up between the SYN and our accept. The next connection must still be
    // served, which is the whole claim.
    harness.incoming.fail(io::ErrorKind::ConnectionAborted);
    let mut client = harness.incoming.connect(1);

    let response = ping(&mut client, "mc.justchunks.net").await;
    assert!(response.body.contains("mc.justchunks.net"), "{response:?}");
}

#[tokio::test]
async fn a_finished_connection_does_not_end_the_server() {
    let harness = demo(None);

    for port in 1..=3 {
        let mut client = harness.incoming.connect(port);
        ping(&mut client, "mc.justchunks.net").await;
    }

    assert!(!harness.server.is_finished());
}

#[tokio::test]
async fn a_panicking_connection_is_logged_loudly_rather_than_vanishing() {
    // The one outcome that used to escape reporting entirely: the connection task unwound, the join
    // handle was dropped, and nothing downstream ever heard about it. It is the driver's own record
    // that closes that hole now, so the driver's own record is what this reads -- and at `warn`,
    // because a handler panicking is ours, not the peer's.
    let (logs, _guard) = record_logs();
    let (tx, rx) = mpsc::unbounded_channel();

    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            make_with(|| PanickingDispatcher),
            |_: &SocketAddr| (),
        )
        .run(),
    );

    let incoming = Incoming(tx);
    let mut client = incoming.connect(1);
    client.send_raw(&[0x00]).await;
    client.expect_eof().await;

    let (level, text) = logs.find("connection failed");
    assert_eq!(level, Level::WARN, "{text}");
    assert!(text.contains("kind=\"panic\""), "{text}");
    assert!(text.contains("a handler bug"), "{text}");

    // And the server carries on: one connection's bug is not the listener's problem.
    assert!(!server.is_finished());
    server.abort();
}

/// A dispatcher that panics on the first frame, standing in for a handler bug.
struct PanickingDispatcher;

impl passage_driver::conn::Dispatcher<()> for PanickingDispatcher {
    fn set_version(&mut self, _version: passage_driver::version::ProtocolVersion) {}

    fn dispatch(&self, _ctx: Ctx<'_, ()>, _id: i32, _payload: &[u8]) -> Result<()> {
        panic!("a handler bug");
    }

    fn tick(&self, _ctx: Ctx<'_, ()>) -> Result<()> {
        Ok(())
    }

    fn ticks(&self) -> bool {
        false
    }
}

#[tokio::test]
async fn a_refused_peer_never_reaches_the_protocol_and_the_limiter_counts_its_own() {
    // Rate limiting as a layer, in the only place it can see the address a PROXY layer reported and
    // still not hold up the accept loop. `&self` is what lets it be a named type with state, so
    // "how many did we turn away" needs no reporting hook at all -- and the driver, which has
    // nothing useful to say about someone else's policy, says nothing.
    let (logs, _guard) = record_logs();
    let (tx, rx) = mpsc::unbounded_channel();
    let counts = Arc::new(Counts::default());

    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            Arc::new(router().expect("builds")),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .layer(RateLimiter {
            blocked_port: 2,
            counts: Arc::clone(&counts),
        })
        .run(),
    );

    let incoming = Incoming(tx);
    let mut refused = incoming.connect(2);
    let mut allowed = incoming.connect(1);

    refused.expect_eof().await;
    assert!(
        ping(&mut allowed, "mc.justchunks.net")
            .await
            .body
            .contains("mc.justchunks.net")
    );

    assert_eq!(counts.refused.load(Ordering::Relaxed), 1);
    assert_eq!(counts.admitted.load(Ordering::Relaxed), 1);

    // One connection ran, so there is one ending on the record -- the refusal is not a connection
    // that ended, and the driver invents nothing to say about it.
    let endings = logs
        .events()
        .into_iter()
        .filter(|(_, text)| text.contains("connection ended") || text.contains("connection closed"))
        .count();
    assert_eq!(endings, 1, "{:#?}", logs.events());

    server.abort();
}

#[tokio::test]
async fn layers_run_in_the_order_they_are_written() {
    // Only one order can produce this result, which is the point of asserting it this way: the
    // rewrite turns port 2 into 1002, and the closure after it refuses 1002. Written the other way
    // round the closure would see port 2, admit it, and the connection would run.
    //
    // The closure is also the `Fn(&Addr) -> bool` impl doing its job -- an admission check with
    // nothing to count does not need a named type.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            Arc::new(router().expect("builds")),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .layer(RewriteAddr { refuse_port: 0 })
        .layer(|addr: &SocketAddr| addr.port() != 1_002)
        .run(),
    );

    let incoming = Incoming(tx);
    let mut refused = incoming.connect(2);
    refused.expect_eof().await;

    // And a peer the closure does not object to is served, so the refusal above was the closure's
    // doing and not the stack failing outright.
    let mut allowed = incoming.connect(3);
    let response = ping(&mut allowed, "mc.justchunks.net").await;
    assert!(response.body.contains("mc.justchunks.net"), "{response:?}");

    server.abort();
}

#[tokio::test]
async fn a_layer_that_refuses_costs_the_next_peer_nothing() {
    // A layer saying no is the listener's equivalent of a malformed PROXY header or a rejected
    // certificate. It is holding the reason and says nothing to anyone; what the *server* owes is
    // that one refused peer does not cost the next one anything.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            Arc::new(router().expect("builds")),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .layer(RewriteAddr { refuse_port: 2 })
        .run(),
    );

    let incoming = Incoming(tx);
    let mut refused = incoming.connect(2);
    refused.expect_eof().await;

    let mut allowed = incoming.connect(1);
    let response = ping(&mut allowed, "mc.justchunks.net").await;
    assert!(response.body.contains("mc.justchunks.net"), "{response:?}");

    server.abort();
}

#[tokio::test]
async fn a_layer_can_change_the_socket_type_and_the_address_downstream_sees() {
    // Two layers, one of which changes the socket type and one of which rewrites the address --
    // between them, everything a preamble is for. The state factory is handed what the *last* layer
    // left, not what the listener first reported, which is the whole point of the PROXY case.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            Arc::new(peer_echo_router()),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .layer(RewriteAddr { refuse_port: 0 })
        .layer(Upgrade)
        .run(),
    );

    let incoming = Incoming(tx);
    let mut client = incoming.connect(25_565);
    client.version = versions::V1_21;
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;

    // The port the layer left behind, not the one the accept reported.
    let body = client.expect::<StatusResponse>().await.body;
    assert!(body.contains("127.0.0.1:26565"), "{body}");

    server.abort();
}

#[tokio::test(start_paused = true)]
async fn shutdown_stops_accepting_but_waits_for_a_live_connection() {
    let harness = demo(None);
    let mut client = harness.incoming.connect(1);

    // A connection that is live and has been answered, but not closed.
    client.version = versions::V1_21;
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;
    let _ = client.expect::<StatusResponse>().await;

    harness.shutdown.cancel();

    // A connection accepted after the signal is not served, because nothing accepts it.
    let mut late = harness.incoming.connect(2);
    late.version = versions::V1_21;
    late.send(&intention(versions::V1_21, Intent::Status)).await;
    late.send(&StatusRequest).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), late.expect::<StatusResponse>())
            .await
            .is_err(),
        "a connection accepted after the shutdown signal was served",
    );

    // And the server is still waiting, because the first connection is still live and nothing
    // cancelled it.
    assert!(
        tokio::time::timeout(Duration::from_secs(30), harness.server)
            .await
            .is_err(),
        "serve returned while a connection was still live",
    );
}

#[tokio::test(start_paused = true)]
async fn the_drain_timeout_cancels_a_connection_that_will_not_end() {
    let (logs, _guard) = record_logs();
    let harness = demo(Some(Duration::from_secs(10)));
    let mut client = harness.incoming.connect(1);

    // Answered, so the connection is certainly live, and then silent. With no idle deadline
    // configured, only the drain timeout ends it.
    client.version = versions::V1_21;
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;
    let _ = client.expect::<StatusResponse>().await;

    harness.shutdown.cancel();

    tokio::time::timeout(Duration::from_secs(60), harness.server)
        .await
        .expect("the drain timeout ends the wait")
        .expect("the accept loop does not panic");

    // Cancelled, not timed out: the drain gave up on it, which is a different thing from the
    // connection's own deadline expiring and the record has to say which.
    let (level, text) = logs.find("connection ended");
    assert_eq!(level, Level::DEBUG, "{text}");
    assert!(text.contains(r#"ending="cancelled""#), "{text}");
}
