//! The test suite the integration tests are written in.
//!
//! A test says what the two sides route and what it expects to have happened. Everything between --
//! the socket pair, the two connections, the state, the endings -- is here, so that a test reads as
//! a conversation rather than as a setup.
//!
//! | Piece                   | For                                                            |
//! |-------------------------|----------------------------------------------------------------|
//! | [`Scenario`]            | two routers meeting over a socket pair, and both endings       |
//! | [`Harness`]             | a real [`Server`](passage_core::server::Server) on a channel   |
//! | [`RawClient`]           | bytes a router cannot express: bad IDs, lying length prefixes  |
//! | [`Notes`]               | what a side saw, as the connection's own state                 |
//! | [`record_logs`]         | what an operator would see, for the endings only the log has   |
//! | [`packets`]             | a small protocol shaped like the real one                      |
//!
//! Each test binary compiles this module separately, so not every one of them uses all of it.

#![allow(dead_code, unused_imports)]

mod harness;
mod logs;
mod notes;
pub mod packets;
mod raw;
mod scenario;

pub use harness::{Accepted, Harness, Incoming, Peer, Running, TestListener, TestServer};
pub use logs::{Logs, record_logs};
pub use notes::Notes;
pub use raw::RawClient;
pub use scenario::{Meeting, Scenario, Served, Side, TestClient, client};

use futures::future::BoxFuture;
use packets::{Handshake, Intent};
use passage_core::connection::{Conn, ConnRef, DispatchError};
use passage_core::router::{Handler, Router, RouterBuilder};
use passage_core::{Packet, Phase};
use std::fmt::Debug;
use std::time::Duration;

/// Starts a router whose state is a set of [`Notes`].
pub fn router() -> RouterBuilder<Notes> {
    Router::<Notes>::builder()
}

/// Registration without the unwrapping.
///
/// A router that does not build is a bug in the test rather than a case under test, so the two
/// tests that *are* about registration call [`RouterBuilder::on`] directly and everything else
/// says what it routes and moves on.
pub trait Routes: Sized {
    /// Registers a handler for `P` that has nothing to wait for.
    ///
    /// Most handlers are this shape, so the test suite hands them the connection directly rather
    /// than making every one of them write a `with`.
    fn handle<P: Packet>(
        self,
        handler: impl Fn(&mut Conn<Notes>, P) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self;

    /// Registers a handler for `P` that may await.
    fn handle_async<P: Packet, H>(self, handler: H) -> Self
    where
        H: for<'a> Handler<'a, Notes, P>;

    /// Registers a handler that records the packet and does nothing else.
    ///
    /// This is what most client routers want: the test asserts on what arrived, in order, without
    /// a handler body that says the same thing three times.
    fn note<P: Packet + Debug>(self) -> Self {
        self.handle::<P>(|conn, packet| {
            conn.state.push(format!("{packet:?}"));
            Ok(())
        })
    }

    /// Registers a handler that records the packet and then ends the connection.
    fn note_and_close<P: Packet + Debug>(self) -> Self {
        self.handle::<P>(|conn, packet| {
            conn.state.push(format!("{packet:?}"));
            conn.close();
            Ok(())
        })
    }
}

impl Routes for RouterBuilder<Notes> {
    fn handle<P: Packet>(
        self,
        handler: impl Fn(&mut Conn<Notes>, P) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self {
        self.handle_async(move |conn: ConnRef<'_, Notes>, packet: P| {
            std::future::ready(conn.with(|conn| handler(conn, packet)))
        })
    }

    fn handle_async<P: Packet, H>(self, handler: H) -> Self
    where
        H: for<'a> Handler<'a, Notes, P>,
    {
        self.on(handler)
            .unwrap_or_else(|error| panic!("the test router should build: {error}"))
    }
}

/// Wraps an open hook written against the connection directly. The hook itself is handed a
/// [`ConnRef`] because it is free to await; the ones that do not are written like this.
pub fn opens(
    hook: impl Fn(&mut Conn<Notes>) -> Result<(), DispatchError> + Send + Sync + 'static,
) -> impl for<'a> Fn(ConnRef<'a, Notes>) -> std::future::Ready<Result<(), DispatchError>>
+ Send
+ Sync
+ 'static {
    move |conn: ConnRef<'_, Notes>| std::future::ready(conn.with(|conn| hook(conn)))
}

/// A keep-alive loop, as an open hook: the shape a driver's clock takes.
pub fn keeps_alive(
    every: Duration,
) -> impl for<'a> Fn(ConnRef<'a, Notes>) -> BoxFuture<'a, Result<(), DispatchError>>
+ Send
+ Sync
+ 'static {
    move |conn: ConnRef<'_, Notes>| {
        Box::pin(async move {
            loop {
                tokio::time::sleep(every).await;
                // Only once the connection has something to wait for.
                if conn.phase() == Phase::Configuration {
                    conn.send(packets::KeepAlive { id: 1 })?;
                }
            }
        })
    }
}

/// The phase an intent leads to.
pub fn phase_of(intent: Intent) -> Phase {
    match intent {
        Intent::Status => Phase::Status,
        Intent::Login => Phase::Login,
    }
}

/// What every server router does first: pin the version the peer asked for, and enter the phase its
/// intent names.
///
/// Both are applied where they are written rather than queued, because the handler holds the
/// connection while it writes them -- so a packet it sends afterwards is encoded against the
/// version it just set, and the frame after this one is routed against the phase it just entered.
pub fn on_handshake(conn: &mut Conn<Notes>, packet: Handshake) -> Result<(), DispatchError> {
    accept(conn, &packet)
}

/// The body of [`on_handshake`], for a handler that has something to add to it.
pub fn accept(conn: &mut Conn<Notes>, packet: &Handshake) -> Result<(), DispatchError> {
    conn.state
        .push(format!("handshake {} {:?}", packet.host, packet.intent));
    conn.set_version(packet.version);
    conn.set_phase(phase_of(packet.intent));
    Ok(())
}

/// Sends the handshake and follows it into the phase the intent names.
///
/// The dialling side speaks first, so this is what a client's open hook is for -- without one it
/// sends nothing and waits forever.
pub fn greet(conn: &mut Conn<Notes>, intent: Intent) -> Result<(), DispatchError> {
    let version = conn.version();
    conn.send(Handshake::new(version, intent))?;
    conn.set_phase(phase_of(intent));
    Ok(())
}

/// The open hook for a client that has nothing to add to its greeting.
pub fn opening(
    intent: Intent,
) -> impl for<'a> Fn(ConnRef<'a, Notes>) -> std::future::Ready<Result<(), DispatchError>>
+ Send
+ Sync
+ 'static {
    opens(move |conn| greet(conn, intent))
}
