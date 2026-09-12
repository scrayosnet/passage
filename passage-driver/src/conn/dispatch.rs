//! What a connection needs from a router.
//!
//! This is the seam, declared by the side that *uses* it: a [`Connection`](super::Connection)
//! depends on this trait, not on [`Router`](crate::router::Router), and
//! [`RouterDispatcher`](crate::router::RouterDispatcher) is the implementation that ships with the
//! crate. No type in [`conn`](super) names the router, and nothing here imports it.
//!
//! Three things fall out of that:
//!
//! * The dispatch table is the dispatcher's business. A connection holds no table, resolves no
//!   packet ID and knows nothing about phases-to-IDs -- it hands over an ID and a payload.
//! * A connection can be driven by something that is not a router: a recorder that replays a
//!   captured session, a test double that asserts on what it was handed, a proxy that forwards
//!   everything it does not understand.
//! * The cost is a virtual call per packet where there used to be a static one. That is the
//!   cheapest thing on the path -- a fraction of what decoding the frame costs -- and it buys the
//!   two properties above.

use crate::conn::{Ctx, Ending};
use crate::error::Result;
use crate::version::ProtocolVersion;

/// Decides what runs for an incoming packet, on a tick, and when the connection ends.
///
/// Implementors own whatever routing state they need. The connection calls
/// [`set_version`](Dispatcher::set_version) when the handshake pins a version and then only ever
/// hands over frames, so an implementation is free to resolve IDs once and cache the result -- see
/// [`RouterDispatcher`](crate::router::RouterDispatcher), which resolves the table for the bound
/// version once and holds the router behind an `Arc`.
pub trait Dispatcher<S> {
    /// Binds dispatch to a protocol version, for every frame after it.
    ///
    /// Called once by [`Connection::builder`](super::Connection::builder) with the initial version,
    /// and
    /// again for every [`Op::SetVersion`](super::Op::SetVersion). Cheap enough to call more often
    /// than that.
    fn set_version(&mut self, version: ProtocolVersion);

    /// Decodes one frame and runs whatever handles it.
    ///
    /// `payload` is the frame with its packet ID already stripped. Returning an error ends the
    /// connection, classified by [`Class`](crate::error::Class) as usual -- an unknown packet is
    /// the peer's fault, a decode that overruns is too. [`on_error`](Dispatcher::on_error) gets the
    /// last word before the socket goes away.
    fn dispatch(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()>;

    /// Runs the tick handler. Doing nothing is a valid implementation.
    fn tick(&self, ctx: Ctx<'_, S>) -> Result<()>;

    /// Whether [`tick`](Dispatcher::tick) does anything.
    ///
    /// A connection leaves its timer unarmed when this is `false`, so a dispatcher without a tick
    /// handler costs no wakeups at all.
    fn ticks(&self) -> bool;

    /// Runs when the connection is ending for a reason nobody asked for.
    ///
    /// This is where a disconnect message comes from. Without it the only thing a failing
    /// connection could do was close the socket, and the peer saw "Internal Exception" for a
    /// missed keep-alive, an unsupported version, a shutdown and a decoding bug alike.
    ///
    /// # What it is called for, and what it is not
    ///
    /// Exactly the [`Ending`]s -- which is to say, exactly the `Err` half of
    /// [`Outcome::result`](super::Outcome::result), and that is not a coincidence: "the connection
    /// did not run its course" and "the dispatcher is asked to answer for it" are the same
    /// condition, so they are the same type. It is *not* called when a handler ended the connection
    /// itself, nor after the peer hung up, where there is nobody to talk to. It runs at most once
    /// per connection.
    ///
    /// It is a last word, not a veto. Whether the connection ends was decided before it was
    /// called, and an error it returns is logged rather than reported -- the ending the connection
    /// already had is the one worth knowing about, not the fact that the apology would not encode.
    /// **Recovering from a failure is the failing handler's job**, and it is in a far better place
    /// to do it: it knows which packet it was reading and what it wanted. A handler that decides an
    /// error is survivable returns `Ok(())`; a router that does not care about packets it cannot
    /// route sets [`UnknownPolicy::Ignore`](crate::router::UnknownPolicy::Ignore).
    ///
    /// # What it may do
    ///
    /// Everything a handler may do, except wait: the loop has stopped, so nothing is dispatched
    /// after it and no task can interleave with what it queues. Operations queued here *are*
    /// written -- the final flush is bounded by
    /// [`ConnectionConfig::close_timeout`](super::ConnectionConfig::close_timeout) rather than by
    /// the shutdown token, so a disconnect still reaches a peer that is shutting us down.
    fn on_error(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()> {
        let _ = (ctx, ending);
        Ok(())
    }
}

// So that a dispatcher chosen at runtime -- `Box<dyn Dispatcher<S>>` -- is itself a dispatcher, the
// same way a boxed `Cipher` is a cipher.
impl<S, D: Dispatcher<S> + ?Sized> Dispatcher<S> for Box<D> {
    fn set_version(&mut self, version: ProtocolVersion) {
        (**self).set_version(version);
    }

    fn dispatch(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        (**self).dispatch(ctx, id, payload)
    }

    fn tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        (**self).tick(ctx)
    }

    fn ticks(&self) -> bool {
        (**self).ticks()
    }

    fn on_error(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()> {
        (**self).on_error(ctx, ending)
    }
}
