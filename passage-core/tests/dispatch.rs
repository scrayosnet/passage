//! Tests for the seam between a connection and whatever routes for it.
//!
//! Nothing here builds a [`Router`](passage_core::router::Router). The point of
//! [`Dispatcher`] being a trait is that a connection depends on the trait and not on the router, so
//! this file drives a connection with a dispatcher that is thirty lines of test code -- and if that
//! ever stops compiling, the dependency has crept back in.

mod common;

use bytes::Bytes;
use common::*;
use futures::future::BoxFuture;
use passage_core::client::{Client, Connected};
use passage_core::connection::{
    ConnRef, DispatchError, Dispatcher, MakeDispatcher, Options, Outcome, make_with,
};
use passage_core::{Phase, ProtocolVersion, versions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// What a dispatcher was asked to do, in order.
#[derive(Debug, Default, PartialEq, Eq)]
struct Log {
    opened: Vec<ProtocolVersion>,
    versions: Vec<ProtocolVersion>,
    frames: Vec<(i32, usize)>,
    keep_alives: usize,
}

/// A dispatcher with no router behind it: it records what it is handed, and on the frame at
/// `close_after` it ends the connection.
#[derive(Clone)]
struct Recorder {
    log: Arc<Mutex<Log>>,
    close_after: usize,
    keep_alive: Option<Duration>,
}

impl Recorder {
    fn new(close_after: usize) -> (Arc<Mutex<Log>>, Self) {
        let log = Arc::new(Mutex::new(Log::default()));
        (
            Arc::clone(&log),
            Self {
                log,
                close_after,
                keep_alive: None,
            },
        )
    }

    /// The same recorder, with a keep-alive of its own every `every`.
    fn every(mut self, every: Duration) -> Self {
        self.keep_alive = Some(every);
        self
    }
}

impl Dispatcher<()> for Recorder {
    fn on_open<'a>(&self, conn: ConnRef<'a, ()>) -> BoxFuture<'a, Result<(), DispatchError>> {
        {
            let mut log = self.log.lock().expect("not poisoned");
            log.opened.push(conn.version());
        }

        // A clock of the connection's own: a handler that runs as long as the connection does.
        let Some(every) = self.keep_alive else {
            return Box::pin(std::future::ready(Ok(())));
        };
        let log = Arc::clone(&self.log);
        Box::pin(async move {
            loop {
                tokio::time::sleep(every).await;
                log.lock().expect("not poisoned").keep_alives += 1;
            }
        })
    }

    fn on_version(&mut self, conn: ConnRef<'_, ()>) -> Result<(), DispatchError> {
        self.log
            .lock()
            .expect("not poisoned")
            .versions
            .push(conn.version());
        Ok(())
    }

    fn on_frame<'a>(
        &self,
        conn: ConnRef<'a, ()>,
        id: i32,
        payload: Bytes,
    ) -> BoxFuture<'a, Result<(), DispatchError>> {
        let seen = {
            let mut log = self.log.lock().expect("not poisoned");
            log.frames.push((id, payload.len()));
            log.frames.len()
        };
        let close_after = self.close_after;

        // Proves the ordinary connection vocabulary is available to any dispatcher, not only to
        // handlers a router happens to hold.
        conn.with(|c| {
            if seen == 1 {
                c.set_version(versions::V26_3);
                c.set_phase(Phase::Status);
            }
            if seen >= close_after {
                c.close();
            }
        });
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Runs a connection over a socket pair with `dispatcher` in the router's place.
fn connect<M: MakeDispatcher<()>>(
    dispatcher: M,
    config: Options,
) -> (RawClient, JoinHandle<Outcome<()>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let connection = Client::new(Connected::new(server_io, Side::Server))
        .state(|_: &Side| ())
        .dispatch(dispatcher)
        .config(config);
    (
        RawClient::new(client_io),
        tokio::spawn(async move { connection.connect().await.expect("preconnected") }),
    )
}

#[tokio::test]
async fn a_connection_runs_on_a_dispatcher_that_is_not_a_router() {
    let (log, recorder) = Recorder::new(2);
    let (mut peer, server) = connect(make_with(move || recorder.clone()), Options::default());

    // Frames the connection cannot possibly know how to route: it forwards the ID and the payload
    // and nothing else.
    peer.send_raw(0x07, &[0x01, 0x02]).await;
    peer.send_raw(0x09, &[]).await;
    peer.expect_eof().await;

    let outcome = server.await.expect("no panic");
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(
        *log.lock().expect("not poisoned"),
        Log {
            // Opened at the configured version, then what the dispatcher itself asked for. The
            // first version is the one it starts at, told before anything is dispatched.
            opened: vec![ProtocolVersion::UNKNOWN],
            versions: vec![ProtocolVersion::UNKNOWN, versions::V26_3],
            // The payload leads with the ID, so a two byte body is three bytes here.
            frames: vec![(7, 3), (9, 1)],
            keep_alives: 0,
        },
    );
}

#[tokio::test]
async fn a_dispatcher_is_opened_at_the_version_the_connection_starts_with() {
    // The hook a client needs: it knows its version before it says anything, so nothing ever queues
    // a version change and this is the only place the dispatcher hears about it.
    let (log, recorder) = Recorder::new(1);
    let (mut peer, server) = connect(
        make_with(move || recorder.clone()),
        Options {
            initial_version: versions::V26_3,
            initial_phase: Phase::Login,
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    peer.expect_eof().await;
    let _ = server.await.expect("no panic");

    assert_eq!(
        log.lock().expect("not poisoned").opened,
        vec![versions::V26_3]
    );
}

#[tokio::test(start_paused = true)]
async fn a_clock_is_the_dispatchers_to_keep_and_the_connection_runs_it() {
    // The connection has no timer beyond its deadline, so a dispatcher that wants one writes it
    // as a handler that keeps running. The deadline ends it whatever it is in the middle of.
    let (log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(
        make_with(move || recorder.clone().every(Duration::from_secs(1))),
        Options {
            max_lifetime: Some(Duration::from_secs(10)),
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    let outcome = server.await.expect("no panic");

    // Ten intervals, give or take the one the deadline lands on.
    let ticked = log.lock().expect("not poisoned").keep_alives;
    assert!((9..=11).contains(&ticked), "ran {ticked} times");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-timeout"),
    );
}

#[tokio::test(start_paused = true)]
async fn a_dispatcher_that_wants_no_clock_is_given_none() {
    // Nothing in the connection ticks by itself.
    let (log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(
        make_with(move || recorder.clone()),
        Options {
            max_lifetime: Some(Duration::from_secs(30)),
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    let _ = server.await.expect("no panic");
    assert_eq!(log.lock().expect("not poisoned").keep_alives, 0);
}

#[tokio::test]
async fn a_dispatcher_can_be_chosen_at_runtime() {
    // The whole point of the `Box<D>` impl: which dispatcher runs is a value, not a type -- and a
    // `Box` that forwarded only some of the hooks would be a silent bug in the one place a
    // dispatcher most needs to hear from the connection.
    let (log, recorder) = Recorder::new(1);
    let (mut peer, server) = connect(
        make_with(move || Box::new(recorder.clone()) as Box<dyn Dispatcher<()> + Send>),
        Options {
            initial_version: versions::V1_20_5,
            ..Options::default()
        },
    );

    peer.send_raw(0x05, &[]).await;
    peer.expect_eof().await;
    let _ = server.await.expect("no panic");

    let log = log.lock().expect("not poisoned");
    assert_eq!(log.frames, vec![(5, 1)]);
    assert_eq!(
        log.opened,
        vec![versions::V1_20_5],
        "through the box as well"
    );
    assert_eq!(log.versions, vec![versions::V1_20_5, versions::V26_3]);
}

#[tokio::test]
async fn a_handler_failure_is_what_the_connection_reports() {
    // A handler that cannot answer the peer says so by failing, and the label it chose is what the
    // outcome and the metrics are keyed by.
    #[derive(Clone)]
    struct Refuses;

    impl Dispatcher<()> for Refuses {
        fn on_frame<'a>(
            &self,
            _conn: ConnRef<'a, ()>,
            _id: i32,
            _payload: Bytes,
        ) -> BoxFuture<'a, Result<(), DispatchError>> {
            Box::pin(std::future::ready(Err(DispatchError::peer(
                "the_real_reason",
                anyhow::anyhow!("what actually went wrong"),
            ))))
        }
    }

    let (mut peer, server) = connect(make_with(|| Refuses), Options::default());
    peer.send_raw(0x00, &[]).await;

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "the_real_reason");
    assert!(error.is_peer_error());
}

#[tokio::test]
async fn a_peer_that_vanishes_mid_flight_ends_the_connection() {
    // A client drops while work it asked for is still running. Nothing is dispatched again, and the
    // hangup is what the outcome reports.
    let (_log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(make_with(move || recorder.clone()), Options::default());

    peer.send_raw(0x00, &[]).await;
    tokio::task::yield_now().await;
    drop(peer);

    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-closed"),
    );
}

#[tokio::test(start_paused = true)]
async fn a_handler_can_answer_a_shutdown_before_the_connection_ends_on_it() {
    // The connection ends on a cancelled token, but a handler awaiting the token gets there
    // first, with the phase and version in hand.
    async fn farewell(conn: ConnRef<'_, ()>) -> Result<(), DispatchError> {
        let shutdown = conn.with(|c| c.shutdown().clone());
        shutdown.cancelled().await;
        conn.with(|c| {
            // Without this the cancellation that woke us also refuses the write.
            c.detach();
            c.set_phase(Phase::Login);
            c.send(packets::Disconnect::text("restarting"))?;
            c.fail(DispatchError::peer(
                "said_goodbye",
                anyhow::anyhow!("restarting"),
            ));
            Ok(())
        })
    }

    struct Farewell;

    impl Dispatcher<()> for Farewell {
        fn on_open<'a>(&self, conn: ConnRef<'a, ()>) -> BoxFuture<'a, Result<(), DispatchError>> {
            Box::pin(farewell(conn))
        }
    }

    let (server_io, client_io) = tokio::io::duplex(4096);
    let shutdown = CancellationToken::new();
    let connection = Client::new(Connected::new(server_io, Side::Server))
        .state(|_: &Side| ())
        .dispatch(make_with(|| Farewell))
        .graceful_shutdown(shutdown.clone());
    let server = tokio::spawn(async move { connection.connect().await.expect("preconnected") });

    let mut peer = RawClient::new(client_io);
    shutdown.cancel();

    let goodbye = peer.expect::<packets::Disconnect>().await;
    assert_eq!(goodbye.reason, "restarting");
    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("said_goodbye"),
    );
}
