//! The operation queue, the connection handle and the handler context.
//!
//! See the [module docs](crate::conn) for why a handler queues operations instead of mutating
//! anything.

use crate::codec::{Cipher, Encoded};
use crate::error::{Error, Result};
use crate::packet::{Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// An operation for the connection to carry out, in queue order.
///
/// This is the whole vocabulary a handler has. If something is not expressible as an `Op`, a
/// handler cannot do it.
pub enum Op<S> {
    /// Write an already-encoded packet, if the connection is still in the version it was encoded
    /// for.
    Send {
        /// The ID varint and payload, plus the packet name for tracing.
        encoded: Encoded,
        /// The protocol version the bytes were encoded for.
        version: ProtocolVersion,
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

    /// Carry out every operation in order, with nothing from anywhere else in between.
    Batch(Vec<Op<S>>),

    /// Fail the connection with this error, after writing everything queued before it.
    ///
    /// The error travels as an operation so that it lands *after* the packets a handler queued
    /// rather than instead of them.
    Fail(Error),

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
            Op::Batch(ops) => write!(f, "Batch({ops:?})"),
            Op::Fail(err) => write!(f, "Fail({err})"),
            Op::Close => f.write_str("Close"),
        }
    }
}

/// A group of operations that reaches the connection as one.
///
/// The queue is shared: a background task holding a [`ConnectionHandle`] can send into it at any
/// moment, so two calls in a row are not guaranteed to be adjacent. Anything whose *adjacency*
/// matters -- "this disconnect message, then close", "switch phase, then send" -- belongs in a
/// batch, which the connection carries out with nothing interleaved.
///
/// Built by [`ConnectionHandle::batch`]; nothing is queued if the closure that fills it fails.
///
/// Two operations are deliberately absent. [`set_version`](ConnectionHandle::set_version), because
/// a batch encodes everything for the version it was created with, so a version change inside one
/// could not affect the packets around it and would only look as if it did. And
/// [`fail`](ConnectionHandle::fail), because a handler that wants to say something and then give up
/// can send and return `Err` -- the connection writes what was queued before the failure either
/// way.
pub struct Batch<S> {
    ops: Vec<Op<S>>,
    version: ProtocolVersion,
    limits: Limits,
}

impl<S> Batch<S> {
    /// Encodes a packet and adds it to the batch. See [`ConnectionHandle::send`].
    pub fn send<P: Packet>(&mut self, packet: P) -> Result<&mut Self> {
        let encoded = Encoded::of(&packet, self.version, self.limits)?;
        Ok(self.push(Op::Send {
            encoded,
            version: self.version,
        }))
    }

    /// Adds an encryption switch. See [`ConnectionHandle::encrypt`].
    pub fn encrypt(&mut self, cipher: Box<dyn Cipher>) -> &mut Self {
        self.push(Op::Encrypt(cipher))
    }

    /// Adds a phase change. See [`ConnectionHandle::set_phase`].
    pub fn set_phase(&mut self, phase: Phase) -> &mut Self {
        self.push(Op::SetPhase(phase))
    }

    /// Adds a change to the connection state. See [`ConnectionHandle::update`].
    pub fn update(&mut self, change: impl FnOnce(&mut S) + Send + 'static) -> &mut Self {
        self.push(Op::With(Box::new(change)))
    }

    /// Ends the batch by closing the connection. See [`ConnectionHandle::close`].
    pub fn close(&mut self) -> &mut Self {
        self.push(Op::Close)
    }

    fn push(&mut self, op: Op<S>) -> &mut Self {
        self.ops.push(op);
        self
    }
}

/// A cheap, cloneable handle to a live connection.
///
/// This is what a handler keeps when it moves work into a background task. Every method is an
/// encode plus a channel send, so it is safe to hold across await points and cannot block.
///
/// The handle carries the protocol version it was created for. The version is pinned once, by the
/// handshake, and the connection re-stamps its own handle when that happens -- so a handle taken
/// from a [`Ctx`] is always current, and one taken from
/// [`ConnectionBuilder::build`](crate::conn::ConnectionBuilder::build) is not (it predates the
/// handshake, and is
/// meant for [`close`](ConnectionHandle::close) and [`shutdown`](ConnectionHandle::shutdown) rather
/// than for sending).
///
/// Everything in it is held *by value*: an `UnboundedSender` and a `CancellationToken` are already
/// cheap clones, so wrapping them in a shared allocation bought nothing and cost an indirection on
/// every queued operation.
pub struct ConnectionHandle<S> {
    ops: mpsc::UnboundedSender<Op<S>>,
    shutdown: CancellationToken,
    version: ProtocolVersion,
    limits: Limits,
}

// Derived `Clone` would demand `S: Clone`, which is wrong: the state is never cloned, only shared.
impl<S> Clone for ConnectionHandle<S> {
    fn clone(&self) -> Self {
        Self {
            ops: self.ops.clone(),
            shutdown: self.shutdown.clone(),
            version: self.version,
            limits: self.limits,
        }
    }
}

impl<S> ConnectionHandle<S> {
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
    /// The encode happens here, not on the connection: an ID that does not exist in this version, a
    /// gated field left unset, or a packet too large for a frame is reported to the code that made
    /// the mistake instead of surfacing later as a connection failure with no obvious author.
    ///
    /// The version travels with the bytes, and the connection refuses to write them if it has moved
    /// on since -- see [`InternalError::StaleEncoding`](crate::error::InternalError::StaleEncoding).
    pub fn send<P: Packet>(&self, packet: P) -> Result<()> {
        let encoded = Encoded::of(&packet, self.version, self.limits)?;
        self.queue(Op::Send {
            encoded,
            version: self.version,
        })
    }

    /// Queues an encryption switch, applying to every byte after the packets queued so far.
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.queue(Op::Encrypt(cipher))
    }

    /// Queues the protocol version.
    ///
    /// It decides which IDs are used for packets queued after it and which decode table the connection
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
    /// the peer's next frame is decoded against the table the handler intended.
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        self.queue(Op::SetPhase(phase))
    }

    /// Queues a change to the connection state.
    ///
    /// # The closure
    ///
    /// It runs on the connection, with `&mut S`, while the queue is being drained. So it must not block
    /// and cannot await. It *may* queue further operations through a cloned handle.
    pub fn update(&self, change: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.queue(Op::With(Box::new(change)))
    }

    /// Runs `f` against the connection state and returns its result.
    ///
    /// This is how a background task *reads* state it does not own. It resolves once the connection has
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

    /// Queues several operations so that nothing can land between them.
    ///
    /// Nothing is queued at all if `build` fails, so a batch is all-or-nothing on both sides:
    ///
    /// ```ignore
    /// ctx.batch(|batch| {
    ///     batch.send(Disconnect { reason })?;
    ///     batch.close();
    ///     Ok(())
    /// })
    /// ```
    pub fn batch(&self, build: impl FnOnce(&mut Batch<S>) -> Result<()>) -> Result<()> {
        let mut batch = Batch {
            ops: Vec::new(),
            version: self.version,
            limits: self.limits,
        };
        build(&mut batch)?;
        self.queue(Op::Batch(batch.ops))
    }

    /// Runs a future *on the connection's own task*, while packets keep being dispatched.
    ///
    /// Use this for work that must overlap with further protocol traffic -- Passage's backend
    /// selection, which runs while keep-alives are exchanged. Anything it needs from the connection
    /// it takes through a cloned [`ConnectionHandle`]. An error from the future fails the connection.
    ///
    /// # It is not a `tokio::spawn`
    ///
    /// The future is polled by the connection loop, between packets. That is what makes
    /// [`exclusive`](ConnectionHandle::exclusive) possible and what keeps the ordering guarantees
    /// intact -- and it means a future that **blocks, or does not yield, stops the connection**: no
    /// frames are read, no keep-alive is sent, no deadline fires. Work that might block belongs in
    /// [`detach`](ConnectionHandle::detach).
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
    /// The connection reopens the gate when the future resolves, so there is nothing to reset by hand
    /// and no way to leave the connection gated by accident. Like
    /// [`spawn`](ConnectionHandle::spawn), it runs on the connection's task.
    pub fn exclusive(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        self.queue(Op::Spawn {
            future: Box::pin(future),
            exclusive: true,
        })
    }

    /// Runs a future on its own task, where it cannot starve the connection.
    ///
    /// The counterpart to [`spawn`](ConnectionHandle::spawn), for work that may block or take a
    /// long slice of CPU: a synchronous resolver, a signature check, an adapter whose client is not
    /// cooperative. It talks back through this handle like any other task, and an error from it
    /// fails the connection through [`fail`](ConnectionHandle::fail).
    ///
    /// What it cannot do is hold the read gate: [`exclusive`](ConnectionHandle::exclusive) counts
    /// tasks the connection itself polls, and this is not one of them.
    ///
    /// Unlike everything else here it returns nothing, because there is nothing that can go wrong
    /// at this end: the task is spawned whether or not the connection is still there, and a task
    /// reporting to a connection that has ended is the ordinary case rather than a failure.
    ///
    /// # Panics
    ///
    /// If called outside a Tokio runtime, like [`tokio::spawn`] itself.
    pub fn detach(&self, future: impl Future<Output = Result<()>> + Send + 'static)
    where
        S: Send + 'static,
    {
        let conn = self.clone();
        tokio::spawn(async move {
            if let Err(err) = future.await {
                let _ = conn.fail(err);
            }
        });
    }

    /// Waits until everything queued so far has been written to the socket.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::Flush(tx))?;
        rx.await.map_err(|_| Error::Closed)
    }

    /// Fails the connection, after writing everything queued so far.
    ///
    /// This is for code that has no `Err` to return: a [`detach`](ConnectionHandle::detach)ed task,
    /// or anything else holding a handle outside a handler call. Inside a handler, returning `Err`
    /// does the same thing and reads better -- the packets queued before it are written either way.
    pub fn fail(&self, error: Error) -> Result<()> {
        self.queue(Op::Fail(error))
    }

    /// Ends the connection once everything queued so far has been written.
    ///
    /// This is a normal completion, not an error: it is how a status response, a transfer and a
    /// refused login all end a connection.
    ///
    /// *Why* it ended is not the driver's to record. A handler that turns a peer away knows the
    /// reason, and the state it writes it into comes back in
    /// [`Outcome`](super::Outcome) -- with the reason attached, which a completion variant could
    /// never carry. Closing and then returning `Ok(())` is also how a handler that has already sent
    /// its own disconnect message declines the one
    /// [`Dispatcher::on_error`](super::Dispatcher::on_error) would otherwise add.
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
    pub conn: &'a ConnectionHandle<S>,

    phase: Phase,
}

impl<'a, S> Ctx<'a, S> {
    pub(crate) fn new(state: &'a S, conn: &'a ConnectionHandle<S>, phase: Phase) -> Self {
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

    /// Encodes a packet and queues it. See [`ConnectionHandle::send`].
    pub fn send<P: Packet>(&self, packet: P) -> Result<()> {
        self.conn.send(packet)
    }

    /// Queues an encryption switch. See [`ConnectionHandle::encrypt`].
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.conn.encrypt(cipher)
    }

    /// Queues the protocol version. See [`ConnectionHandle::set_version`].
    pub fn set_version(&self, version: ProtocolVersion) -> Result<()> {
        self.conn.set_version(version)
    }

    /// Queues a phase change. See [`ConnectionHandle::set_phase`].
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        self.conn.set_phase(phase)
    }

    /// Queues a change to the connection state. See [`ConnectionHandle::update`].
    pub fn update(&self, change: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.conn.update(change)
    }

    /// Queues several operations with nothing in between. See [`ConnectionHandle::batch`].
    pub fn batch(&self, build: impl FnOnce(&mut Batch<S>) -> Result<()>) -> Result<()> {
        self.conn.batch(build)
    }

    /// Runs a future on the connection's task. See [`ConnectionHandle::spawn`].
    pub fn spawn(&self, future: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.conn.spawn(future)
    }

    /// Runs a future, requiring the peer to stay quiet. See [`ConnectionHandle::exclusive`].
    pub fn exclusive(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        self.conn.exclusive(future)
    }

    /// Runs a future on its own task. See [`ConnectionHandle::detach`].
    pub fn detach(&self, future: impl Future<Output = Result<()>> + Send + 'static)
    where
        S: Send + 'static,
    {
        self.conn.detach(future);
    }

    /// Fails the connection in queue order. See [`ConnectionHandle::fail`].
    pub fn fail(&self, error: Error) -> Result<()> {
        self.conn.fail(error)
    }

    /// Ends the connection once everything queued so far has been written.
    pub fn close(&self) -> Result<()> {
        self.conn.close()
    }
}
