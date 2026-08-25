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
//! before the first round:
//!   bind the dispatcher to the version the connection starts at
//!   dispatch on_open
//!
//! each round:
//!   rebind if a handler moved the version, re-arm if one moved the deadline
//!   write the outbox, then flush
//!   stop if the connection is ending
//!   biased select:
//!     1. a finished handler future, or output a running one queued
//!     2. shutdown
//!     3. the lifetime deadline
//!     4. the next frame  (dispatched, polled once, deferred only if it parks)
//! ```
//!
//! A handler that never awaits finishes at dispatch, before the next frame is read -- so the phase
//! and version it set are already true for the packet after it.
//!
//! # There is no clock
//!
//! Beyond its deadline, the connection has no timer. A driver that wants one writes it as a handler
//! that keeps running: a loop in [`Dispatcher::on_open`] that sleeps, sends a keep-alive, and sleeps
//! again. What such a handler queues is written in the round it queued it, so it does not wait for
//! the peer to say something first.
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
//! [`Conn::close`] ends the connection with nothing to report, [`Conn::fail`] ends it with a
//! reason, and both take effect once everything queued before them has been written. A hangup, a
//! cancelled token, an expired deadline and a handler that returns an error all end it the same
//! way; the first failure is what the [`Outcome`] reports.
//!
//! Nothing is dispatched for the ending itself. A disconnect message is sent *before* it, by a
//! handler that awaits [`Conn::shutdown`] or wakes before [`Conn::deadline`] -- it is the only
//! place that knows the phase, the version and the locale to send one in. Such a handler
//! [detaches](Conn::detach) from the token, or pushes the deadline out, so that what it is
//! answering does not cut off what it writes.

mod connection;
mod dispatch;
mod error;
mod handle;

pub use connection::*;
pub use dispatch::*;
pub use error::*;
pub use handle::*;
