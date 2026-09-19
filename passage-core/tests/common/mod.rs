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

use packets::{Handshake, Intent};
use passage_core::connection::{Ctx, DispatchError};
use passage_core::router::{Router, RouterBuilder};
use passage_core::{Packet, Phase};
use std::fmt::Debug;

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
    /// Registers a handler for `P`.
    fn handle<P: Packet>(
        self,
        handler: impl Fn(Ctx<'_, Notes>, P) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self;

    /// Registers a handler that records the packet and does nothing else.
    ///
    /// This is what most client routers want: the test asserts on what arrived, in order, without
    /// a handler body that says the same thing three times.
    fn note<P: Packet + Debug>(self) -> Self {
        self.handle::<P>(|ctx, packet| {
            ctx.state.push(format!("{packet:?}"));
            Ok(())
        })
    }

    /// Registers a handler that records the packet and then ends the connection.
    fn note_and_close<P: Packet + Debug>(self) -> Self {
        self.handle::<P>(|ctx, packet| {
            ctx.state.push(format!("{packet:?}"));
            ctx.handle.close()?;
            Ok(())
        })
    }
}

impl Routes for RouterBuilder<Notes> {
    fn handle<P: Packet>(
        self,
        handler: impl Fn(Ctx<'_, Notes>, P) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self {
        self.on::<P>(handler)
            .unwrap_or_else(|error| panic!("the test router should build: {error}"))
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
/// Both are queued rather than applied, so the packets a handler sends after this are still encoded
/// against the version the *handler* saw -- which is the guard the connection makes good on.
pub fn on_handshake(ctx: Ctx<'_, Notes>, packet: Handshake) -> Result<(), DispatchError> {
    accept(&ctx, &packet)
}

/// The body of [`on_handshake`], for a handler that has something to add to it.
pub fn accept(ctx: &Ctx<'_, Notes>, packet: &Handshake) -> Result<(), DispatchError> {
    ctx.state
        .push(format!("handshake {} {:?}", packet.host, packet.intent));
    ctx.handle.batch(|batch| {
        batch.set_version(packet.version);
        batch.set_phase(phase_of(packet.intent));
        Ok(())
    })?;
    Ok(())
}

/// Sends the handshake and follows it into the phase the intent names.
///
/// The dialling side speaks first, so this is what a client's open hook is for -- without one it
/// sends nothing and waits forever.
pub fn greet(ctx: &Ctx<'_, Notes>, intent: Intent) -> Result<(), DispatchError> {
    ctx.handle.batch(|batch| {
        batch.send(ctx.version, Handshake::new(ctx.version, intent))?;
        batch.set_phase(phase_of(intent));
        Ok(())
    })?;
    Ok(())
}

/// The open hook for a client that has nothing to add to its greeting.
pub fn opening(intent: Intent) -> impl Fn(Ctx<'_, Notes>) -> Result<(), DispatchError> {
    move |ctx| greet(&ctx, intent)
}
