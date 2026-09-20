//! One connection: the socket, the state, and the operation queue that changes them.
//!
//! This is the per-connection half of the crate. [`Router`](crate::router::Router) is the other
//! half: built once at startup, immutable, shared. Nothing here is shared with anything, which is
//! why none of it needs a lock.
//!
//! | Type           | Lifetime              | Owns                                            |
//! |----------------|-----------------------|-------------------------------------------------|
//! | [`Connection`] | one accepted socket   | the socket, the codec, the timers               |
//! | [`ConnCell`]   | one run of a socket   | the state, the phase, the version, the outbox   |
//! | [`ConnRef`]    | borrowed by a handler | nothing -- it is a `Copy` borrow of the cell    |
//! | [`Conn`]       | one `with` closure    | what the handler is lent for the body of it     |
//!
//! # A handler is lent the connection, synchronously
//!
//! A handler reaches the connection through [`ConnRef::with`], whose closure is **not** async. So
//! everything inside one `with` is indivisible with respect to every other handler, and no handler
//! can be holding the connection while it waits on anything. "Record the profile, then announce it"
//! and "send this, then switch to encryption" mean what they say, without a queue and without a
//! lock that anyone could hold across an `.await`.
//!
//! State, phase and version are applied where they are written. Only two things have to wait for
//! the loop, because only two of them need the socket: see [`Out`].
//!
//! # A handler cannot escape
//!
//! [`ConnRef`] is deliberately not `'static`, so it cannot be moved into a [`tokio::spawn`]. Every
//! writer is therefore a future the connection itself is driving -- which is what lets the ending
//! below stop all of them by dropping one task set, and what makes the cell's lock provably
//! uncontended.
//!
//! # The loop
//!
//! ```text
//! each round:
//!   rebind if a handler moved the version
//!   write the outbox, then flush
//!   stop if a handler asked to close
//!   biased select:
//!     1. finished handler futures
//!     2. shutdown
//!     3. the lifetime deadline
//!     4. tick            (keep-alives; not while the peer must stay quiet)
//!     5. the next frame  (dispatched, polled once, deferred only if it parks)
//! ```
//!
//! A handler that never awaits finishes at dispatch, before the next frame is read -- so the phase
//! and version it set are already true for the packet after it.
//!
//! # The read gate
//!
//! [`Conn::gate`] marks work the peer is expected to wait for -- an authentication call, a
//! session-server round trip. While it is shut the connection still polls the socket, and treats a
//! frame that arrives anyway as [`ConnectionError::EarlyPacket`] rather than as input to be
//! replayed later. Work that must genuinely overlap with further traffic simply does not call it.
//!
//! # Ending
//!
//! A handler that calls [`Conn::close`] ends the connection with no error, after everything it
//! queued before that has been written. Everything else -- a hangup, a deadline, a cancelled token,
//! a failure -- goes to [`Dispatcher::on_error`], which is where a disconnect message comes from.
//!
//! Before that hook runs, the task set is dropped and the outbox is cleared: no other writer can
//! exist, and what the failing handler queued this round must not precede the last word. The hook
//! gets a clock of its own ([`Options::close_timeout`]) and a fresh token, so neither an expired
//! lifetime nor a cancelled token -- two of the reasons there is something left to say -- can cut
//! it short.

mod connection;
mod dispatch;
mod error;
mod handle;

pub use connection::*;
pub use dispatch::*;
pub use error::*;
pub use handle::*;
