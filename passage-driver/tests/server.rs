//! Tests for the accept loop: per-connection state, shutdown, draining, accept errors.
//!
//! The listener is a channel rather than a socket, so the tests decide exactly when a connection
//! arrives and never touch the network. That [`Listener`] can be implemented over a socket pair at
//! all is part of what is being tested.

mod common;

use common::{TestClient, intention, intention_to};
use passage_driver::conn::{ConnectionConfig, Ctx};
use passage_driver::demo::packets::{
    Intent, Intention, PingRequest, PongResponse, StatusRequest, StatusResponse,
};
use passage_driver::demo::server::{Session, router};
use passage_driver::error::{Class, Result};
use passage_driver::packet::Phase;
use passage_driver::router::Router;
use passage_driver::server::{Admit, Finished, Listener, Server, make_with, serve};
use passage_driver::version::versions;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// What the test hands to the accept loop.
type Accepted = io::Result<(DuplexStream, SocketAddr)>;

/// How a connection ended, in a form a test can compare.
///
/// An [`Ending`] is not `Clone` -- a failure carries its source -- so what gets recorded is the
/// completion, or the ending's label and whatever blame came with it.
type Ended = std::result::Result<(), (Option<Class>, &'static str)>;

/// A listener fed by a channel: the test decides what is accepted, and when.
struct TestListener {
    incoming: mpsc::UnboundedReceiver<Accepted>,
}

impl Listener for TestListener {
    type Io = DuplexStream;
    type Addr = SocketAddr;
    type Pending = DuplexStream;

    async fn accept(&mut self) -> Accepted {
        match self.incoming.recv().await {
            Some(accepted) => accepted,
            // Nothing more is coming. A quiet listener blocks; it does not report an error in a
            // loop, and the accept loop must not treat "quiet" as anything at all.
            None => std::future::pending().await,
        }
    }

    async fn prepare(pending: DuplexStream, _addr: &mut SocketAddr) -> io::Result<DuplexStream> {
        Ok(pending)
    }
}

/// A listener whose preamble refuses one peer, and corrects the address of the rest.
///
/// It is also the shape [`Listener::Pending`] exists for: the rule comes from the listener's own
/// configuration, and reaches `prepare` -- which never sees `&self` -- only because `accept` put it
/// in the pending value.
struct PreambleListener {
    incoming: mpsc::UnboundedReceiver<Accepted>,
    refuse_port: u16,
}

impl Listener for PreambleListener {
    type Io = DuplexStream;
    type Addr = SocketAddr;
    type Pending = (DuplexStream, u16);

    async fn accept(&mut self) -> io::Result<((DuplexStream, u16), SocketAddr)> {
        let (io, addr) = match self.incoming.recv().await {
            Some(accepted) => accepted?,
            None => std::future::pending().await,
        };
        Ok(((io, self.refuse_port), addr))
    }

    async fn prepare(
        (io, refuse_port): (DuplexStream, u16),
        addr: &mut SocketAddr,
    ) -> io::Result<DuplexStream> {
        if addr.port() == refuse_port {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad preamble"));
        }
        // As a PROXY header would: what the rest of the server sees is what this leaves behind.
        addr.set_port(addr.port() + 1_000);
        Ok(io)
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
    finished: Arc<Mutex<Vec<Ended>>>,
    server: JoinHandle<()>,
}

impl Harness {
    /// What has been recorded so far.
    fn outcomes(&self) -> Vec<Ended> {
        self.finished.lock().expect("not poisoned").clone()
    }

    /// Waits for `count` connections to have finished, so assertions do not race the tasks that
    /// end them.
    async fn wait_for(&self, count: usize) -> Vec<Ended> {
        for _ in 0..1_000 {
            let outcomes = self.outcomes();
            if outcomes.len() >= count {
                return outcomes;
            }
            tokio::task::yield_now().await;
        }
        panic!(
            "only {} of {count} connections finished",
            self.outcomes().len(),
        );
    }
}

/// Flattens a report into something a test can compare.
fn ended<S>(finished: &Finished<'_, S, SocketAddr>) -> Ended {
    match finished.result {
        Ok(()) => Ok(()),
        Err(ending) => Err((
            ending.error().map(passage_driver::error::Error::class),
            ending.label(),
        )),
    }
}

/// Starts a server on a channel-backed listener.
fn start(router: Router<Session>, drain: Option<Duration>) -> Harness {
    let (tx, rx) = mpsc::unbounded_channel();
    let shutdown = CancellationToken::new();
    let finished = Arc::new(Mutex::new(Vec::new()));

    let recorder = Arc::clone(&finished);
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
    .graceful_shutdown(shutdown.clone())
    // Annotated because the bound is `Report`, not `Fn`: a closure's parameter can only be
    // inferred from a bound that names `Fn` directly. `on_finish(log_completion)` needs nothing.
    .on_finish(move |finished: &Finished<'_, Session, SocketAddr>| {
        recorder.lock().expect("not poisoned").push(ended(finished));
    });

    if let Some(after) = drain {
        server = server.drain_timeout(after);
    }

    Harness {
        incoming: Incoming(tx),
        shutdown,
        finished,
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
    assert_eq!(harness.wait_for(1).await, vec![Ok(())]);
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

    assert_eq!(harness.wait_for(3).await.len(), 3);
    assert!(!harness.server.is_finished());
}

#[tokio::test]
async fn a_panicking_connection_is_reported_rather_than_vanishing() {
    // The one outcome that used to escape reporting entirely: the connection task unwound, the
    // join handle was dropped, and nothing downstream ever heard about it.
    let (tx, rx) = mpsc::unbounded_channel();
    let finished: Arc<Mutex<Vec<Ended>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&finished);

    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            make_with(|| PanickingDispatcher),
            |_: &SocketAddr| (),
        )
        .on_finish(move |finished: &Finished<'_, (), SocketAddr>| {
            recorder.lock().expect("not poisoned").push(ended(finished));
        })
        .run(),
    );

    let incoming = Incoming(tx);
    let mut client = incoming.connect(1);
    client.send_raw(&[0x00]).await;

    for _ in 0..1_000 {
        if !finished.lock().expect("not poisoned").is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        *finished.lock().expect("not poisoned"),
        vec![Err((Some(Class::Internal), "panic"))],
    );
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

/// What a limiter publishes, which is the only thing anyone else needs from it.
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

impl Admit<SocketAddr> for RateLimiter {
    fn admit(&self, addr: &SocketAddr) -> bool {
        let allowed = addr.port() != self.blocked_port;
        // Counted here, at the decision, where the reason is still in hand.
        let counter = if allowed {
            &self.counts.admitted
        } else {
            &self.counts.refused
        };
        counter.fetch_add(1, Ordering::Relaxed);
        allowed
    }
}

#[tokio::test]
async fn a_refused_peer_never_reaches_the_protocol_and_the_limiter_counts_its_own() {
    // Rate limiting, in the only place it can see the address a PROXY header reported and still not
    // hold up the accept loop. `&self` is what lets it be a named type with state, so "how many did
    // we turn away" needs no reporting hook at all.
    let (tx, rx) = mpsc::unbounded_channel();
    let counts = Arc::new(Counts::default());
    let limiter = RateLimiter {
        blocked_port: 2,
        counts: Arc::clone(&counts),
    };
    let finished: Arc<Mutex<Vec<Ended>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&finished);

    let server = tokio::spawn(
        serve(
            TestListener { incoming: rx },
            Arc::new(router().expect("builds")),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .on_accept(limiter)
        .on_finish(move |finished: &Finished<'_, Session, SocketAddr>| {
            recorder.lock().expect("not poisoned").push(ended(finished));
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

    // Only the connection that ran is reported. The refusal is not a connection that ended, and the
    // report has no arm for it.
    assert_eq!(finished.lock().expect("not poisoned").clone(), vec![Ok(())]);
    assert_eq!(counts.refused.load(Ordering::Relaxed), 1);
    assert_eq!(counts.admitted.load(Ordering::Relaxed), 1);

    server.abort();
}

#[tokio::test]
async fn a_failed_preamble_closes_the_socket_and_the_server_carries_on() {
    // `prepare` failing is the listener's business: it holds the TLS error or the malformed header
    // and can count it where that is still true. What the server owes is that one peer's bad
    // preamble costs the next peer nothing.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        serve(
            PreambleListener {
                incoming: rx,
                refuse_port: 2,
            },
            Arc::new(router().expect("builds")),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
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
async fn a_preamble_can_correct_the_address_everything_downstream_sees() {
    // What `Listener::prepare` exists for, and why it takes the address by `&mut`: a PROXY header
    // reports the real client, and the state factory and admission check must see that one.
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(
        serve(
            PreambleListener {
                incoming: rx,
                refuse_port: 0,
            },
            Arc::new(peer_echo_router()),
            |addr| Session {
                peer: Some(*addr),
                ..Session::default()
            },
        )
        .run(),
    );

    let incoming = Incoming(tx);
    let mut client = incoming.connect(25_565);
    client.version = versions::V1_21;
    client
        .send(&intention(versions::V1_21, Intent::Status))
        .await;
    client.send(&StatusRequest).await;

    // The port the preamble left behind, not the one the accept reported.
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

    let Harness {
        finished, server, ..
    } = harness;
    tokio::time::timeout(Duration::from_secs(60), server)
        .await
        .expect("the drain timeout ends the wait")
        .expect("the accept loop does not panic");

    assert_eq!(
        *finished.lock().expect("not poisoned"),
        vec![Err((None, "cancelled"))],
    );
}
