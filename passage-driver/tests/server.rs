//! Tests for the accept loop: per-connection state, shutdown, draining, accept errors.
//!
//! The listener is a channel rather than a socket, so the tests decide exactly when a connection
//! arrives and never touch the network. That [`Listener`] can be implemented over a socket pair at
//! all is part of what is being tested.

mod common;

use common::{TestClient, intention, intention_to};
use passage_driver::conn::{Completion, ConnectionConfig, Ctx};
use passage_driver::demo::packets::{
    Intent, Intention, PingRequest, PongResponse, StatusRequest, StatusResponse,
};
use passage_driver::demo::server::{SUPPORTED_VERSIONS, Session, router};
use passage_driver::error::{Class, Result};
use passage_driver::packet::{Direction, Phase};
use passage_driver::router::Router;
use passage_driver::server::{Listener, serve};
use passage_driver::version::versions;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
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
/// [`Result<Completion>`] is not `Clone` -- an error carries its source -- so what gets recorded is
/// the completion, or who was to blame and what for.
type Outcome = std::result::Result<Completion, (Class, &'static str)>;

/// A listener fed by a channel: the test decides what is accepted, and when.
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
    finished: Arc<Mutex<Vec<Outcome>>>,
    server: JoinHandle<()>,
}

impl Harness {
    /// What has been recorded so far.
    fn outcomes(&self) -> Vec<Outcome> {
        self.finished.lock().expect("not poisoned").clone()
    }

    /// Waits for `count` connections to have finished, so assertions do not race the tasks that
    /// end them.
    async fn wait_for(&self, count: usize) -> Vec<Outcome> {
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

/// Starts a server on a channel-backed listener.
fn start(router: Router<Session>, drain: Option<Duration>) -> Harness {
    let (tx, rx) = mpsc::unbounded_channel();
    let shutdown = CancellationToken::new();
    let finished = Arc::new(Mutex::new(Vec::new()));

    let recorder = Arc::clone(&finished);
    let mut server = serve(
        TestListener { incoming: rx },
        router,
        // The point of a factory rather than a value: the address is only known per connection.
        |addr: &SocketAddr| Session {
            peer: Some(*addr),
            ..Session::default()
        },
    )
    .config(ConnectionConfig::default())
    .with_graceful_shutdown(shutdown.clone())
    .on_finish(move |result: &Result<Completion>| {
        let outcome = match result {
            Ok(completion) => Ok(*completion),
            Err(err) => Err((err.class(), err.label())),
        };
        recorder.lock().expect("not poisoned").push(outcome);
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
    assert_eq!(harness.wait_for(1).await, vec![Ok(Completion::Closed)]);
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

#[tokio::test]
async fn the_state_factory_sees_the_address_the_listener_reported() {
    // Two handlers, so the assertion is on the state and nothing else.
    let peer_router = Router::builder(Direction::Serverbound)
        .on::<Intention, _>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Status)
        })
        .on::<StatusRequest, _>(|ctx: Ctx<'_, Session>, _: StatusRequest| {
            ctx.send(StatusResponse {
                body: format!("{:?}", ctx.state.peer),
            })?;
            ctx.close()
        })
        .build(SUPPORTED_VERSIONS.iter().copied())
        .expect("builds");

    let harness = start(peer_router, None);
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
        vec![Ok(Completion::Cancelled)],
    );
}
