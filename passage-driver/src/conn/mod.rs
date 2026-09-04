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
//!   4. deadlines           (lifetime, idle)
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

mod connection;
mod handle;

pub use connection::{Completion, Connection, ConnectionConfig};
pub use handle::{ConnectionHandle, Ctx, Op};
