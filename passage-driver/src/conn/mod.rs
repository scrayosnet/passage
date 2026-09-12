//! One connection: the socket, the state, and the operation queue that changes them.
//!
//! This is the per-connection half of the crate. [`Router`](crate::router::Router) is the other
//! half: built once at startup, immutable, shared by every connection. Nothing here is shared with
//! anything, which is why none of it needs a lock:
//!
//! | Type                | Lifetime                | Owns                                     |
//! |---------------------|-------------------------|------------------------------------------|
//! | [`Connection`]      | one accepted socket     | the socket, the state, the phase, the version |
//! | [`ConnectionHandle`]| cloned, outlives handlers | the operation queue and the shutdown token |
//! | [`Ctx`]             | one handler call        | nothing -- a borrow plus a snapshot      |
//!
//! [`Connection`] does four things and nothing else -- frame, dispatch, tick, and shut down. All
//! protocol logic lives in the router's handlers, and everything a handler wants to happen it
//! queues as an [`Op`]. To accept sockets and run one [`Connection`] per socket, see
//! [`server::serve`](crate::server::serve).
//!
//! # It does not know about the router
//!
//! Dispatch is reached through the [`Dispatcher`] trait, which this module declares and
//! [`router`](crate::router) implements. So the dependency points one way only -- nothing in here
//! mentions [`Router`](crate::router::Router) -- and a connection holds no dispatch table, resolves
//! no packet ID and needs no rebinding logic of its own. What it hands over is a frame; what comes
//! back is a [`Result`](crate::error::Result).
//!
//! # Everything a handler does is an operation
//!
//! A handler holds nothing and mutates nothing. It reads a snapshot of the connection through
//! [`Ctx`] and *queues* whatever it wants to happen -- a packet, a state change, a phase change,
//! an encryption switch, a background task, a close -- as an [`Op`]. The connection drains that
//! queue with priority, in order, with exclusive access to everything it owns.
//!
//! Three properties fall out of that, and none of them needs a lock:
//!
//! * **One writer.** The connection is the only thing that touches the socket, the state, the phase
//!   and the version. Nothing can interleave, not even a packet queued from a background task.
//! * **Ordered side effects.** "Record the profile, then announce it" and "send this, then switch
//!   to encryption" mean what they say, because both halves are operations in one queue. Getting
//!   the second one wrong is the classic "works until the client is slow" bug; getting the first
//!   one wrong gives you a session whose login has been announced but not recorded.
//! * **No stale reads.** [`Ctx::phase`] and [`Ctx::version`] are values the connection passed in,
//!   not atomics that another task may already have moved on from.
//!
//! The cost is that a handler cannot observe its own effects: `ctx.send(..)` then `ctx.state` still
//! shows the old state. That is the point -- the alternative is a handler that half-applied its
//! changes before returning an error.
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
//! The priority order is the design. Operations first means a handler's effects -- the packets it
//! queued, the phase it moved to, the state it changed -- are all applied before the next packet is
//! even looked at. That is what makes an operation queue equivalent to exclusive access without a
//! lock, and it is what makes the read gate below sound: by the time a frame is considered, every
//! [`Op::Spawn`] that preceded it has been counted.
//!
//! # The read gate
//!
//! [`ConnectionHandle::exclusive`] marks work the peer is expected to wait for -- an authentication
//! call, a session-server round trip. While such a task is in flight the connection still **polls**
//! the socket, and treats what it finds as a protocol break rather than as input to be replayed
//! later:
//!
//! * a frame is [`ProtocolError::EarlyPacket`](crate::error::ProtocolError::EarlyPacket) -- a
//!   compliant peer had nothing to send;
//! * an EOF ends the connection immediately, instead of after the adapter call returns on a
//!   connection nobody is on the other end of any more.
//!
//! Work that must genuinely overlap with further traffic uses [`ConnectionHandle::spawn`] instead.
//! That is the difference between "the framework decided to run my handlers concurrently" and "I
//! asked for concurrency here".
//!
//! Both of those run on the connection's own task, which is what makes the gate and the ordering
//! work -- and what makes a future that blocks stop the connection.
//! [`ConnectionHandle::detach`] is the escape hatch for work that might.
//!
//! # Ending
//!
//! A connection ends in one of two ways, and the difference is **who decided**. That is the
//! `Result` in [`Outcome::result`], so there is one place to look and nothing to translate:
//!
//! * **`Ok(())` -- we did.** A handler queued [`Op::Close`]. There is nothing to report and nothing
//!   left to do, which is also how a handler that has already sent its own disconnect message
//!   declines the one `on_error` would add: close, and return `Ok(())`.
//! * **`Err(`[`Ending`]`)` -- something else did.** The peer hung up, a deadline expired, the
//!   shutdown token was cancelled, or something failed. All four go to [`Dispatcher::on_error`],
//!   which takes an `Ending` for that reason and no other.
//!
//! A hangup belongs on the second side even though nobody did anything wrong. The question the
//! split answers is not "was this a failure" -- [`Ending::error`] answers that, and says no for
//! three of the four -- but "did we finish what we were doing". A client that disappears while its
//! backend is being selected has left a selection running, and releasing it is the same job as
//! releasing it after a timeout. `on_error` is the one place that job can live.
//!
//! Either way, everything already queued is written before the socket closes: a handler that
//! queues a disconnect and *then* fails gets both. What bounds that final stretch is
//! [`ConnectionConfig::close_timeout`], not the shutdown token -- a cancelled token is one of the
//! reasons there is something to say.
//!
//! # Sequence is the handler's business
//!
//! The connection knows about phases, not about steps. Within one phase the protocol is a
//! sequence -- login start, then cookie response, then encryption response -- and nothing here
//! enforces it: a peer may send the third packet first and the router will dispatch it. Handlers
//! that care check [`Ctx::state`] and refuse what does not fit, which is the same place the rest
//! of the session's facts live. See `demo::server` for the shape.

mod connection;
mod dispatch;
mod handle;

pub use connection::{Connection, ConnectionBuilder, ConnectionConfig, Ending, Outcome};
pub use dispatch::Dispatcher;
pub use handle::{Batch, ConnectionHandle, Ctx, Op};
