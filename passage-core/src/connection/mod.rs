//! One connection: the socket, the state, and the operation queue that changes them.
//!
//! This is the per-connection half of the crate. [`Router`](crate::router::Router) is the other
//! half: built once at startup, immutable, shared. Nothing here is shared with anything, which is
//! why none of it needs a lock.
//!
//! | Type                 | Lifetime                  | Owns                                          |
//! |----------------------|---------------------------|-----------------------------------------------|
//! | [`Connection`]       | one accepted socket       | the socket, the state, the phase, the version |
//! | [`ConnectionHandle`] | cloned, outlives handlers | the operation queue and the shutdown token    |
//! | [`Ctx`]              | one handler call          | nothing -- a borrow plus a snapshot           |
//!
//! # Everything a handler does is an operation
//!
//! A handler holds nothing and mutates nothing. It reads a snapshot of the connection through
//! [`Ctx`] and *queues* whatever it wants to happen as an [`Op`]. The connection drains that queue
//! first, in order, with exclusive access to everything it owns. So "record the profile, then
//! announce it" and "send this, then switch to encryption" mean what they say, and no lock is
//! needed to make them.
//!
//! The cost is that a handler cannot observe its own effects: `send` then `ctx.state` still shows
//! the old state. That is the point -- the alternative is a handler that half-applied its changes
//! before returning an error.
//!
//! # The loop
//!
//! ```text
//! biased select:
//!   1. queued operations   (writes, state, phase, version, spawn, close)  <- drained first
//!   2. finished handler tasks
//!   3. shutdown
//!   4. the lifetime deadline
//!   5. tick                (keep-alives; not while the peer must stay quiet)
//!   6. the next frame
//! ```
//!
//! # The read gate
//!
//! [`ConnectionHandle::exclusive`] marks work the peer is expected to wait for -- an authentication
//! call, a session-server round trip. While such a task is in flight the connection still polls the
//! socket, and treats a frame that arrives anyway as
//! [`ConnectionError::EarlyPacket`] rather than as input to be replayed later. Work that must
//! genuinely overlap with further traffic uses [`ConnectionHandle::spawn`]; work that might block
//! uses [`ConnectionHandle::detach`].
//!
//! # Ending
//!
//! A handler that queues [`Op::Close`] ends the connection with no error. Everything else -- a
//! hangup, a deadline, a cancelled token, a failure -- goes to
//! [`Dispatcher::on_error`], which is where a disconnect message comes from. That hook gets a fresh
//! operation queue and a clock of its own ([`Options::close_timeout`]), so nothing a detached task
//! still holds can interleave with the last thing we say, and neither an expired lifetime nor a
//! cancelled token can cut it short.

mod connection;
mod dispatch;
mod error;
mod handle;

pub use connection::*;
pub use dispatch::*;
pub use error::*;
pub use handle::*;
