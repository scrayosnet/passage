//! The operation queue, the connection handle and the handler context.
//!
//! # Everything a handler does is an operation
//!
//! A handler holds nothing and mutates nothing. It reads a snapshot of the connection through
//! [`Ctx`] and *queues* whatever it wants to happen -- a packet, a state change, a phase change,
//! an encryption switch, a background task, a close -- as an [`Op`]. The driver drains that queue
//! with priority, in order, with exclusive access to everything it owns.
//!
//! Three properties fall out of that, and none of them needs a lock:
//!
//! * **One writer.** The driver is the only thing that touches the socket, the state, the phase and
//!   the version. Nothing can interleave, not even a packet queued from a background task.
//! * **Ordered side effects.** "Record the profile, then announce it" and "send this, then switch
//!   to encryption" mean what they say, because both halves are operations in one queue. Getting
//!   the second one wrong is the classic "works until the client is slow" bug; getting the first
//!   one wrong gives you a session whose login has been announced but not recorded.
//! * **No stale reads.** [`Ctx::phase`] and [`Ctx::version`] are values the driver passed in, not
//!   atomics that another task may already have moved on from.
//!
//! The cost is that a handler cannot observe its own effects: `ctx.send(..)` then `ctx.state` still
//! shows the old state. That is the point -- the alternative is a handler that half-applied its
//! changes before returning an error.
//!
//! # Two types, not three
//!
//! [`ConnHandle`] holds the queue and the connection's configuration *by value*: an
//! `UnboundedSender` and a `CancellationToken` are already cheap clones, so wrapping them in a
//! shared allocation bought nothing and cost an indirection on every queued operation. [`Ctx`] is
//! then a borrow of a handle plus the two things only the driver can supply -- the state snapshot
//! and the phase.

use crate::codec::{Cipher, Encoded};
use crate::error::{Error, Result};
use crate::packet::{Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// An operation for the driver to carry out, in queue order.
///
/// This is the whole vocabulary a handler has. If something is not expressible as an `Op`, a
/// handler cannot do it.
pub enum Op<S> {
    /// Write an already-encoded packet, if the connection is still in the configuration it was
    /// encoded for.
    Send {
        /// The ID varint and payload, plus the packet name for tracing.
        encoded: Encoded,
        /// The protocol version the bytes were encoded for.
        version: ProtocolVersion,
        /// The phase the packet belongs to.
        phase: Phase,
    },

    /// Enable encryption for every byte from here on.
    Encrypt(Box<dyn Cipher>),

    /// Pin the protocol version, which decides the ID table for everything after it.
    SetVersion(ProtocolVersion),

    /// Move the connection to another protocol phase.
    SetPhase(Phase),

    /// Run a closure against the connection state.
    With(Box<dyn FnOnce(&mut S) + Send>),

    /// Run a future alongside the connection, dropping it when the connection ends.
    Spawn {
        /// The work to run.
        future: BoxFuture<'static, Result<()>>,
        /// Whether the peer has to stay quiet until it resolves.
        exclusive: bool,
    },

    /// Flush the socket and notify the waiter once everything queued before has been written.
    Flush(oneshot::Sender<()>),

    /// Finish the connection after writing everything queued before.
    Close,
}

impl<S> std::fmt::Debug for Op<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Op::Send { encoded, .. } => write!(f, "Send({})", encoded.name),
            Op::Encrypt(_) => f.write_str("Encrypt"),
            Op::SetVersion(version) => write!(f, "SetVersion({version})"),
            Op::SetPhase(phase) => write!(f, "SetPhase({phase:?})"),
            Op::With(_) => f.write_str("With"),
            Op::Spawn { exclusive, .. } => write!(f, "Spawn {{ exclusive: {exclusive} }}"),
            Op::Flush(_) => f.write_str("Flush"),
            Op::Close => f.write_str("Close"),
        }
    }
}

/// A cheap, cloneable handle to a live connection.
///
/// This is what a handler keeps when it moves work into a background task. Every method is an
/// encode plus a channel send, so it is safe to hold across await points and cannot block.
///
/// The handle carries the protocol version it was created for. The version is pinned once, by the
/// handshake, and the driver re-stamps its own handle when that happens -- so a handle taken from a
/// [`Ctx`] is always current, and one taken from
/// [`Driver::new`](crate::driver::Driver::new) is not (it predates the handshake, and is meant for
/// [`close`](ConnHandle::close) and [`shutdown`](ConnHandle::shutdown) rather than for sending).
pub struct ConnHandle<S> {
    ops: mpsc::UnboundedSender<Op<S>>,
    shutdown: CancellationToken,
    version: ProtocolVersion,
    limits: Limits,
}

// Derived `Clone` would demand `S: Clone`, which is wrong: the state is never cloned, only shared.
impl<S> Clone for ConnHandle<S> {
    fn clone(&self) -> Self {
        Self {
            ops: self.ops.clone(),
            shutdown: self.shutdown.clone(),
            version: self.version,
            limits: self.limits,
        }
    }
}

impl<S> ConnHandle<S> {
    /// Creates a handle and the receiving end of its operation queue.
    pub(crate) fn new(
        shutdown: CancellationToken,
        version: ProtocolVersion,
        limits: Limits,
    ) -> (Self, mpsc::UnboundedReceiver<Op<S>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                ops: tx,
                shutdown,
                version,
                limits,
            },
            rx,
        )
    }

    /// The same handle, stamped for another protocol version.
    pub(crate) fn at_version(&self, version: ProtocolVersion) -> Self {
        Self {
            version,
            ..self.clone()
        }
    }

    /// The protocol version this handle encodes for.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// The decoding limits of this connection.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Encodes a packet and queues it.
    ///
    /// The encode happens here, not on the driver: an ID that does not exist in this version, a
    /// gated field left unset, or a packet too large for a frame is reported to the code that made
    /// the mistake instead of surfacing later as a connection failure with no obvious author.
    ///
    /// The version and the packet's phase travel with the bytes, and the driver refuses to write
    /// them if the connection has moved on -- see
    /// [`InternalError::StaleEncoding`](crate::error::InternalError::StaleEncoding).
    pub fn send<P: Packet>(&self, packet: P) -> Result<()> {
        let encoded = Encoded::of(&packet, self.version, self.limits)?;
        self.queue(Op::Send {
            encoded,
            version: self.version,
            phase: P::PHASE,
        })
    }

    /// Queues an encryption switch, applying to every byte after the packets queued so far.
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.queue(Op::Encrypt(cipher))
    }

    /// Queues the protocol version.
    ///
    /// It decides which IDs are used for packets queued after it and which decode table the driver
    /// uses for the next frame. In a server this is called once, from the handshake handler.
    ///
    /// A handler that pins the version cannot also send in it: its own view of the version is the
    /// snapshot from before the change, so the send would be refused as a stale encoding. Answer
    /// from the next handler, which sees the pinned version.
    pub fn set_version(&self, version: ProtocolVersion) -> Result<()> {
        self.queue(Op::SetVersion(version))
    }

    /// Queues a phase change.
    ///
    /// Because this is an operation, it lands exactly between the packet queued before it and the
    /// one queued after -- so "send the last packet of this phase, then switch" is expressible, and
    /// the peer's next frame is decoded against the table the handler intended. The reverse order
    /// is the mistake, and it is refused rather than written.
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        self.queue(Op::SetPhase(phase))
    }

    /// Queues a change to the connection state.
    ///
    /// # The closure
    ///
    /// It runs on the driver, with `&mut S`, while the queue is being drained. So it must not block
    /// and cannot await. It *may* queue further operations through a cloned handle.
    pub fn update(&self, change: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.queue(Op::With(Box::new(change)))
    }

    /// Runs `f` against the connection state and returns its result.
    ///
    /// This is how a background task *reads* state it does not own. It resolves once the driver has
    /// drained everything queued before it, so it doubles as a barrier: `conn.with(|_| ()).await`
    /// means "everything I queued has been carried out".
    pub async fn with<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut S) -> R + Send + 'static,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::With(Box::new(move |state| {
            // The receiver having gone away only means nobody is listening anymore.
            let _ = tx.send(f(state));
        })))?;
        rx.await.map_err(|_| Error::Closed)
    }

    /// Runs a future alongside the connection, while packets keep being dispatched.
    ///
    /// Use this for work that must overlap with further protocol traffic -- Passage's backend
    /// selection, which runs while keep-alives are exchanged. Anything it needs from the connection
    /// it takes through a cloned [`ConnHandle`]. An error from the future fails the connection.
    pub fn spawn(&self, future: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.queue(Op::Spawn {
            future: Box::pin(future),
            exclusive: false,
        })
    }

    /// Runs a future and requires the peer to stay quiet until it resolves.
    ///
    /// This is the shape of every step that waits on something external before it can answer: an
    /// authentication call, a session-server round trip, a cookie lookup. A compliant peer is
    /// waiting for that answer, so no frame is due -- and one that arrives anyway is reported as
    /// [`ProtocolError::EarlyPacket`](crate::error::ProtocolError::EarlyPacket) rather than
    /// buffered and replayed into a half-finished session.
    ///
    /// The driver reopens the gate when the future resolves, so there is nothing to reset by hand
    /// and no way to leave the connection gated by accident.
    pub fn exclusive(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        self.queue(Op::Spawn {
            future: Box::pin(future),
            exclusive: true,
        })
    }

    /// Waits until everything queued so far has been written to the socket.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::Flush(tx))?;
        rx.await.map_err(|_| Error::Closed)
    }

    /// Ends the connection once everything queued so far has been written.
    ///
    /// This is a normal completion, not an error: it is how a status response, a disconnect or a
    /// transfer ends a connection.
    pub fn close(&self) -> Result<()> {
        self.queue(Op::Close)
    }

    /// The token that is cancelled when the connection ends.
    #[must_use]
    pub fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }

    fn queue(&self, op: Op<S>) -> Result<()> {
        self.ops.send(op).map_err(|_| Error::Closed)
    }
}

/// What a handler is given: a read-only view of the connection, and the queue to change it.
///
/// The state is `&S`, not `&mut S` and not a lock. Reads are free and cannot observe a half-applied
/// change; writes go through [`Ctx::update`] and land in queue order together with everything else
/// the handler asked for.
pub struct Ctx<'a, S> {
    /// The per-connection state, as of the start of this handler.
    pub state: &'a S,

    /// The connection handle. Clone it into a background task.
    pub conn: &'a ConnHandle<S>,

    phase: Phase,
}

impl<'a, S> Ctx<'a, S> {
    pub(crate) fn new(state: &'a S, conn: &'a ConnHandle<S>, phase: Phase) -> Self {
        Self { state, conn, phase }
    }

    /// The decoding limits of this connection.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.conn.limits()
    }

    /// The negotiated protocol version.
    ///
    /// Capture this into a background task that needs it: it does not change after the handshake.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.conn.version()
    }

    /// The phase the connection is in.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Encodes a packet and queues it. See [`ConnHandle::send`].
    pub fn send<P: Packet>(&self, packet: P) -> Result<()> {
        self.conn.send(packet)
    }

    /// Queues an encryption switch. See [`ConnHandle::encrypt`].
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.conn.encrypt(cipher)
    }

    /// Queues the protocol version. See [`ConnHandle::set_version`].
    pub fn set_version(&self, version: ProtocolVersion) -> Result<()> {
        self.conn.set_version(version)
    }

    /// Queues a phase change. See [`ConnHandle::set_phase`].
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        self.conn.set_phase(phase)
    }

    /// Queues a change to the connection state. See [`ConnHandle::update`].
    pub fn update(&self, change: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.conn.update(change)
    }

    /// Runs a future alongside the connection. See [`ConnHandle::spawn`].
    pub fn spawn(&self, future: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.conn.spawn(future)
    }

    /// Runs a future, requiring the peer to stay quiet. See [`ConnHandle::exclusive`].
    pub fn exclusive(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        self.conn.exclusive(future)
    }

    /// Ends the connection once everything queued so far has been written.
    pub fn close(&self) -> Result<()> {
        self.conn.close()
    }
}
