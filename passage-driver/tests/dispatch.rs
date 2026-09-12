//! Tests for the seam between a connection and whatever routes for it.
//!
//! Nothing here builds a [`Router`](passage_driver::router::Router). The point of
//! [`Dispatcher`] being a trait is that a connection depends on the trait and not on the router, so
//! this file drives a connection with a dispatcher that is 30 lines of test code -- and if that
//! ever stops compiling, the dependency has crept back in.

mod common;

use common::TestClient;
use passage_driver::conn::{
    Connection, ConnectionConfig, ConnectionHandle, Ctx, Dispatcher, Outcome,
};
use passage_driver::error::Result;
use passage_driver::packet::Phase;
use passage_driver::version::{ProtocolVersion, versions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;

/// What a dispatcher was asked to do, in order.
#[derive(Debug, Default, PartialEq, Eq)]
struct Log {
    versions: Vec<ProtocolVersion>,
    frames: Vec<(i32, usize)>,
    ticks: usize,
}

/// A dispatcher with no router behind it: it records what it is handed, and on the frame at
/// `close_after` it ends the connection.
struct Recorder {
    log: Arc<Mutex<Log>>,
    ticks: bool,
    close_after: usize,
}

impl Dispatcher<()> for Recorder {
    fn set_version(&mut self, version: ProtocolVersion) {
        self.log
            .lock()
            .expect("not poisoned")
            .versions
            .push(version);
    }

    fn dispatch(&self, ctx: Ctx<'_, ()>, id: i32, payload: &[u8]) -> Result<()> {
        let seen = {
            let mut log = self.log.lock().expect("not poisoned");
            log.frames.push((id, payload.len()));
            log.frames.len()
        };

        // Proves the ordinary operation vocabulary is available to any dispatcher, not just to
        // handlers a router happens to hold.
        if seen == 1 {
            ctx.set_version(versions::V1_21)?;
            ctx.set_phase(Phase::Status)?;
        }
        if seen >= self.close_after {
            ctx.close()?;
        }
        Ok(())
    }

    fn tick(&self, _ctx: Ctx<'_, ()>) -> Result<()> {
        self.log.lock().expect("not poisoned").ticks += 1;
        Ok(())
    }

    fn ticks(&self) -> bool {
        self.ticks
    }
}

/// Runs a connection over a socket pair with `dispatcher` in the router's place.
fn connect<D: Dispatcher<()> + Send + 'static>(
    dispatcher: D,
    config: ConnectionConfig,
) -> (TestClient, ConnectionHandle<()>, JoinHandle<Outcome<()>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let (connection, handle) = Connection::builder(server_io, dispatcher, ())
        .config(config)
        .build();
    (
        TestClient::new(client_io),
        handle,
        tokio::spawn(connection.run()),
    )
}

fn recorder(ticks: bool, close_after: usize) -> (Arc<Mutex<Log>>, Recorder) {
    let log = Arc::new(Mutex::new(Log::default()));
    let recorder = Recorder {
        log: Arc::clone(&log),
        ticks,
        close_after,
    };
    (log, recorder)
}

#[tokio::test]
async fn a_connection_runs_on_a_dispatcher_that_is_not_a_router() {
    let (log, recorder) = recorder(false, 2);
    let (mut client, _handle, server) = connect(recorder, ConnectionConfig::default());

    // Frames the connection cannot possibly know how to route: it forwards the ID and the payload
    // and nothing else.
    client.send_raw(&[0x07, 0x01, 0x02]).await;
    client.send_raw(&[0x09]).await;
    client.expect_eof().await;

    server
        .await
        .expect("no panic")
        .result
        .expect("closes cleanly");
    assert_eq!(
        *log.lock().expect("not poisoned"),
        Log {
            // UNKNOWN from the configuration, then what the dispatcher itself asked for -- which it
            // learns about the same way a router does, through the connection.
            versions: vec![ProtocolVersion::UNKNOWN, versions::V1_21],
            frames: vec![(7, 2), (9, 0)],
            ticks: 0,
        },
    );
}

#[tokio::test(start_paused = true)]
async fn a_dispatcher_that_does_not_tick_is_never_ticked() {
    let (log, recorder) = recorder(false, usize::MAX);
    let config = ConnectionConfig {
        tick_interval: Some(Duration::from_secs(1)),
        ..ConnectionConfig::default()
    };
    let (mut client, handle, server) = connect(recorder, config);

    client.send_raw(&[0x01]).await;
    tokio::time::sleep(Duration::from_secs(30)).await;

    assert_eq!(log.lock().expect("not poisoned").ticks, 0);
    handle.close().expect("the connection is live");
    server
        .await
        .expect("no panic")
        .result
        .expect("closes cleanly");
}

#[tokio::test(start_paused = true)]
async fn a_dispatcher_that_ticks_is_ticked_on_the_configured_interval() {
    let (log, recorder) = recorder(true, usize::MAX);
    let config = ConnectionConfig {
        tick_interval: Some(Duration::from_secs(1)),
        ..ConnectionConfig::default()
    };
    let (mut client, handle, server) = connect(recorder, config);

    client.send_raw(&[0x01]).await;
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Ten intervals, give or take the one the sleep lands on.
    let ticked = log.lock().expect("not poisoned").ticks;
    assert!((9..=11).contains(&ticked), "ticked {ticked} times");

    handle.close().expect("the connection is live");
    server
        .await
        .expect("no panic")
        .result
        .expect("closes cleanly");
}

#[tokio::test]
async fn a_dispatcher_can_be_chosen_at_runtime() {
    let (log, recorder) = recorder(false, 1);
    // The whole point of the `Box<D>` impl: which dispatcher runs is a value, not a type.
    let boxed: Box<dyn Dispatcher<()> + Send> = Box::new(recorder);
    let (mut client, _handle, server) = connect(boxed, ConnectionConfig::default());

    client.send_raw(&[0x05]).await;
    client.expect_eof().await;

    server
        .await
        .expect("no panic")
        .result
        .expect("closes cleanly");
    assert_eq!(log.lock().expect("not poisoned").frames, vec![(5, 0)]);
}
