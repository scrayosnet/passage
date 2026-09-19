//! Tests for the seam between a connection and whatever routes for it.
//!
//! Nothing here builds a [`Router`](passage_core::router::Router). The point of
//! [`Dispatcher`] being a trait is that a connection depends on the trait and not on the router, so
//! this file drives a connection with a dispatcher that is thirty lines of test code -- and if that
//! ever stops compiling, the dependency has crept back in.

mod common;

use common::*;
use passage_core::client::{Client, Connected};
use passage_core::connection::{
    ConnectionError, Ctx, DispatchError, Dispatcher, MakeDispatcher, Options, Outcome, make_with,
};
use passage_core::{Phase, ProtocolVersion, versions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

/// What a dispatcher was asked to do, in order.
#[derive(Debug, Default, PartialEq, Eq)]
struct Log {
    opened: Vec<ProtocolVersion>,
    versions: Vec<ProtocolVersion>,
    frames: Vec<(i32, usize)>,
    ticks: usize,
    endings: Vec<&'static str>,
}

/// A dispatcher with no router behind it: it records what it is handed, and on the frame at
/// `close_after` it ends the connection.
#[derive(Clone)]
struct Recorder {
    log: Arc<Mutex<Log>>,
    close_after: usize,
}

impl Recorder {
    fn new(close_after: usize) -> (Arc<Mutex<Log>>, Self) {
        let log = Arc::new(Mutex::new(Log::default()));
        (Arc::clone(&log), Self { log, close_after })
    }
}

impl Dispatcher<()> for Recorder {
    fn on_open(&mut self, ctx: Ctx<'_, ()>) -> Result<(), DispatchError> {
        self.log
            .lock()
            .expect("not poisoned")
            .opened
            .push(ctx.version);
        Ok(())
    }

    fn on_version(&mut self, ctx: Ctx<'_, ()>) -> Result<(), DispatchError> {
        self.log
            .lock()
            .expect("not poisoned")
            .versions
            .push(ctx.version);
        Ok(())
    }

    fn on_frame(&self, ctx: Ctx<'_, ()>, id: i32, payload: &[u8]) -> Result<(), DispatchError> {
        let seen = {
            let mut log = self.log.lock().expect("not poisoned");
            log.frames.push((id, payload.len()));
            log.frames.len()
        };

        // Proves the ordinary operation vocabulary is available to any dispatcher, not only to
        // handlers a router happens to hold.
        if seen == 1 {
            ctx.handle.set_version(versions::V26_2)?;
            ctx.handle.set_phase(Phase::Status)?;
        }
        if seen >= self.close_after {
            ctx.handle.close()?;
        }
        Ok(())
    }

    fn on_tick(&self, _ctx: Ctx<'_, ()>) -> Result<(), DispatchError> {
        self.log.lock().expect("not poisoned").ticks += 1;
        Ok(())
    }

    fn on_error(
        &self,
        _ctx: Ctx<'_, ()>,
        error: &mut ConnectionError,
    ) -> Result<(), DispatchError> {
        self.log
            .lock()
            .expect("not poisoned")
            .endings
            .push(error.reason());
        Ok(())
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
            // Opened at the configured version, then what the dispatcher itself asked for -- which
            // it learns about the same way a router does, through the connection.
            opened: vec![ProtocolVersion::UNKNOWN],
            versions: vec![versions::V26_2],
            // The payload leads with the ID, so a two byte body is three bytes here.
            frames: vec![(7, 3), (9, 1)],
            ticks: 0,
            endings: vec![],
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
            initial_version: versions::V26_2,
            initial_phase: Phase::Login,
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    peer.expect_eof().await;
    let _ = server.await.expect("no panic");

    assert_eq!(
        log.lock().expect("not poisoned").opened,
        vec![versions::V26_2]
    );
}

#[tokio::test(start_paused = true)]
async fn a_dispatcher_is_ticked_on_the_configured_interval_and_not_otherwise() {
    let (log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(
        make_with(move || recorder.clone()),
        Options {
            tick_interval: Some(Duration::from_secs(1)),
            max_lifetime: Some(Duration::from_secs(10)),
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    let outcome = server.await.expect("no panic");

    // Ten intervals, give or take the one the deadline lands on.
    let ticked = log.lock().expect("not poisoned").ticks;
    assert!((9..=11).contains(&ticked), "ticked {ticked} times");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-timeout"),
    );
    assert_eq!(
        log.lock().expect("not poisoned").endings,
        vec!["peer-timeout"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_connection_without_an_interval_is_never_ticked() {
    // `tick_interval: None` is the opt-out, and it is the caller's to set.
    let (log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(
        make_with(move || recorder.clone()),
        Options {
            tick_interval: None,
            max_lifetime: Some(Duration::from_secs(30)),
            ..Options::default()
        },
    );

    peer.send_raw(0x01, &[]).await;
    let _ = server.await.expect("no panic");
    assert_eq!(log.lock().expect("not poisoned").ticks, 0);
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
    assert_eq!(log.versions, vec![versions::V26_2]);
}

#[tokio::test]
async fn a_failure_the_hook_cannot_answer_still_reports_what_it_was() {
    // `on_error` is a last word, not a second cause: if it fails, the connection still reports what
    // it was ending for. Reporting the failed apology instead would lose the diagnosis exactly when
    // it is most wanted.
    #[derive(Clone)]
    struct Unhelpful;

    impl Dispatcher<()> for Unhelpful {
        fn on_frame(
            &self,
            _ctx: Ctx<'_, ()>,
            _id: i32,
            _payload: &[u8],
        ) -> Result<(), DispatchError> {
            Err(DispatchError::peer(
                "the_real_reason",
                anyhow::anyhow!("what actually went wrong"),
            ))
        }

        fn on_error(
            &self,
            _ctx: Ctx<'_, ()>,
            _error: &mut ConnectionError,
        ) -> Result<(), DispatchError> {
            Err(DispatchError::internal(
                "the_answer_broke",
                anyhow::anyhow!("and nobody needs to know"),
            ))
        }
    }

    let (mut peer, server) = connect(make_with(|| Unhelpful), Options::default());
    peer.send_raw(0x00, &[]).await;

    let outcome = server.await.expect("no panic");
    let error = outcome.error.expect("must fail the connection");
    assert_eq!(error.reason(), "the_real_reason");
    assert!(error.is_peer_error());
}

#[tokio::test]
async fn a_peer_that_vanishes_mid_flight_still_runs_the_hook() {
    // A client drops while work it asked for is still running. If that ending never reached
    // `on_error`, whatever the work reserved would be reserved forever -- the connection is gone,
    // so no packet is ever dispatched again and no handler could catch it.
    let (log, recorder) = Recorder::new(usize::MAX);
    let (mut peer, server) = connect(make_with(move || recorder.clone()), Options::default());

    peer.send_raw(0x00, &[]).await;
    // Let the frame be dispatched, then vanish.
    tokio::task::yield_now().await;
    drop(peer);

    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.error.map(|error| error.reason()),
        Some("peer-closed"),
    );
    assert_eq!(
        log.lock().expect("not poisoned").endings,
        vec!["peer-closed"]
    );
}
