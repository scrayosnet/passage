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
//! * The cost is a virtual call per packet where there used to be a static one -- 0.9 ns against a
//!   12 ns table lookup, which is why the table stays cached in the dispatcher rather than being
//!   looked up per frame.

use crate::conn::Ctx;
use crate::error::Result;
use crate::version::ProtocolVersion;

/// Decides what runs for an incoming packet, and on a tick.
///
/// Implementors own whatever routing state they need. The connection calls
/// [`set_version`](Dispatcher::set_version) when the handshake pins a version and then only ever
/// hands over frames, so an implementation is free to resolve IDs once and cache the result -- see
/// [`RouterDispatcher`](crate::router::RouterDispatcher), which caches the table for the bound
/// version and holds the router behind an `Arc`.
pub trait Dispatcher<S> {
    /// Binds dispatch to a protocol version, for every frame after it.
    ///
    /// Called once by [`Connection::new`](super::Connection::new) with the initial version, and
    /// again for every [`Op::SetVersion`](super::Op::SetVersion). Cheap enough to call more often
    /// than that.
    fn set_version(&mut self, version: ProtocolVersion);

    /// Decodes one frame and runs whatever handles it.
    ///
    /// `payload` is the frame with its packet ID already stripped. Returning an error ends the
    /// connection, classified by [`Class`](crate::error::Class) as usual -- an unknown packet is
    /// the peer's fault, a decode that overruns is too.
    fn dispatch(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()>;

    /// Runs the tick handler. Doing nothing is a valid implementation.
    fn tick(&self, ctx: Ctx<'_, S>) -> Result<()>;

    /// Whether [`tick`](Dispatcher::tick) does anything.
    ///
    /// A connection leaves its timer unarmed when this is `false`, so a dispatcher without a tick
    /// handler costs no wakeups at all.
    fn ticks(&self) -> bool;
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
}
