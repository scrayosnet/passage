use crate::codec::{Cipher, Frame};
use crate::connection::error::{ConnectionError, Result};
use crate::packet::Packet;
use crate::phase::Phase;
use crate::version::ProtocolVersion;
use crate::wire::Options;
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// An operation is used to mutate the connection state asynchronously without exclusive locks. Handlers
/// get an (unbounded) channel sender to pass operations on.
pub enum Op<S> {
    /// Sends a packet to the peer. The packet is pre-encoded for the specified version.
    Send {
        /// The encoded packet.
        encoded: Frame,

        /// The protocol version, the packet was encoded for.
        version: ProtocolVersion,
    },

    /// Enable encryption for the connection. Can only be applied once.
    Encrypt(Box<dyn Cipher>),

    /// Set the protocol version for the connection. This affects the incoming packet routing.
    SetVersion(ProtocolVersion),

    /// Set the phase for the connection. This affects the incoming packet routing.
    SetPhase(Phase),

    /// Run a closure against the connection state. Using an operation instead of a shared mutex
    /// ensures that the state update is applied in order with the other operations. A shared mutex
    /// state may still be used with its respective tradeoffs (namely out-of-order updates).
    With(Box<dyn FnOnce(&mut S) + Send>),

    /// Run a closure alongside the connection. The future is dropped as soon as the connection ends.
    /// This is *not* a `tokio::spawn` but instead runs in the connection scope.
    Spawn {
        /// The closure to run.
        future: BoxFuture<'static, Result<()>>,

        /// Whether the peer may transmit packets before the future resolves. Setting it to `true`
        /// while the client sends packets results in an error and the connection being closed.
        exclusive: bool,
    },

    /// Flush the socket and every operation queued before it, notifying the channel once this operation
    /// is reached. A similar pattern may be implemented using the [`Op::With`] operation but this is
    /// more explicit.
    Flush(oneshot::Sender<()>),

    /// Execute a batch of operations in order, ensuring that nothing can land between them.
    Batch(Vec<Op<S>>),

    /// Fail the connection with an error, after writing everything queued before it. The error travels
    /// as an operation so that it lands *after* the packets a handler queued rather than instead of them.
    ///
    /// Prefer using the [`Op::Close`] if the error was already handled.
    Fail(ConnectionError),

    /// Finish the connection after writing everything queued before. This does not trigger an error.
    /// It should be used when the connection close was already handled.
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

/// A batch of operations that should be executed in order without interleaving other operations.
///
/// This is helpful if multiple (background) handlers are running at the same time and may interleave
/// their operations. The batch has to be sent with the connection handle, constructing does not send
/// it.
pub struct Batch<S> {
    /// The (ordered) operations to execute.
    ops: Vec<Op<S>>,

    /// The connection's options, used for encoding packets.
    options: Options,
}

impl<S> Batch<S> {
    /// Adds a [`Op::Send`] to the batch, returning itself.
    pub fn send<P: Packet>(&mut self, version: ProtocolVersion, packet: P) -> Result<&mut Self> {
        let encoded = Frame::of(&packet, version, self.options)?;
        Ok(self.push(Op::Send { encoded, version }))
    }

    /// Adds a [`Op::Encrypt`] to the batch, returning itself.
    pub fn encrypt(&mut self, cipher: Box<dyn Cipher>) -> &mut Self {
        self.push(Op::Encrypt(cipher))
    }

    /// Adds a [`Op::SetPhase`] to the batch, returning itself.
    pub fn set_phase(&mut self, phase: Phase) -> &mut Self {
        self.push(Op::SetPhase(phase))
    }

    /// Adds a [`Op::With`] to the batch, returning itself.
    pub fn update(&mut self, change: impl FnOnce(&mut S) + Send + 'static) -> &mut Self {
        self.push(Op::With(Box::new(change)))
    }

    /// Adds a [`Op::Close`] to the batch, returning iself.
    pub fn close(&mut self) -> &mut Self {
        self.push(Op::Close)
    }

    /// Pushes an operation to the batch, returning iself.
    pub fn push(&mut self, op: Op<S>) -> &mut Self {
        self.ops.push(op);
        self
    }
}

/// An inexpensive, cloneable handle to a live connection. It is used to interact with the connection
/// (sync/async). Interactions are queued to the connection using a shared channel. This channel
/// is shared by all handlers and should be cloned and shared.
#[derive(Debug)]
pub struct ConnectionHandle<S> {
    ops: mpsc::UnboundedSender<Op<S>>,
    shutdown: CancellationToken,
    options: Options,
}

impl<S> Clone for ConnectionHandle<S> {
    fn clone(&self) -> Self {
        Self {
            ops: self.ops.clone(),
            shutdown: self.shutdown.clone(),
            options: self.options,
        }
    }
}

impl<S> ConnectionHandle<S> {
    /// Creates a new connection handle with its own (unbounded) channel.
    pub(crate) fn new(
        shutdown: CancellationToken,
        options: Options,
    ) -> (Self, mpsc::UnboundedReceiver<Op<S>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Self {
                ops: tx,
                shutdown,
                options,
            },
            rx,
        )
    }

    /// The wire options of this connection (handle).
    #[must_use]
    pub fn options(&self) -> Options {
        self.options
    }

    /// Queues a [`Op::Send`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn send<P: Packet>(&self, version: ProtocolVersion, packet: P) -> Result<()> {
        let encoded = Frame::of(&packet, version, self.options)?;
        self.queue(Op::Send { encoded, version })
    }

    /// Queues a [`Op::Encrypt`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.queue(Op::Encrypt(cipher))
    }

    /// Queues a [`Op::SetVersion`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn set_version(&self, version: ProtocolVersion) -> Result<()> {
        self.queue(Op::SetVersion(version))
    }

    /// Queues a [`Op::SetPhase`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn set_phase(&self, phase: Phase) -> Result<()> {
        self.queue(Op::SetPhase(phase))
    }

    /// Queues a [`Op::With`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn update(&self, change: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.queue(Op::With(Box::new(change)))
    }

    /// Queues a [`Op::With`] to the connection and waits until the operation completes.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub async fn with<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut S) -> R + Send + 'static,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::With(Box::new(move |state| {
            // The receiver having gone away only means nobody is listening anymore.
            let _ = tx.send(f(state));
        })))?;
        rx.await.map_err(|_| ConnectionError::shutdown())
    }

    /// Queues a [`Op::Batch`] to the connection.
    ///
    /// ```ignore
    /// ctx.batch(|batch| {
    ///     batch.send(Disconnect { reason })?;
    ///     batch.close();
    ///     Ok(())
    /// })
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn batch(&self, build: impl FnOnce(&mut Batch<S>) -> Result<()>) -> Result<()> {
        let mut batch = Batch {
            ops: Vec::new(),
            options: self.options,
        };
        build(&mut batch)?;
        self.queue(Op::Batch(batch.ops))
    }

    /// Queues a [`Op::Spawn`] to the connection without exclusive.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn spawn(&self, future: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.queue(Op::Spawn {
            future: Box::pin(future),
            exclusive: false,
        })
    }

    /// Queues a [`Op::Spawn`] to the connection with exclusive.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn exclusive(
        &self,
        future: impl Future<Output = Result<()>> + Send + 'static,
    ) -> Result<()> {
        self.queue(Op::Spawn {
            future: Box::pin(future),
            exclusive: true,
        })
    }

    /// Queues a [`Op::Spawn`] to the connection with the future running in its own `tokio::spawn`.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
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

    /// Queues a [`Op::Flush`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::Flush(tx))?;
        rx.await.map_err(|_| ConnectionError::shutdown())
    }

    /// Queues a [`Op::Fail`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn fail(&self, error: ConnectionError) -> Result<()> {
        self.queue(Op::Fail(error))
    }

    /// Queues a [`Op::Close`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn close(&self) -> Result<()> {
        self.queue(Op::Close)
    }

    /// Gets the cancellation token for the connection handle (and connection). This should be used
    /// for detached jobs to ensure that they complete.
    #[must_use]
    pub fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }

    /// Queues an [`Op`] to the connection.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Closed`] in case the connection was already closed.
    pub fn queue(&self, op: Op<S>) -> Result<()> {
        self.ops.send(op).map_err(|_| ConnectionError::shutdown())
    }
}

/// The handler context, used to interact with the connection. It contains the connection state (i.e.,
/// state, phase, and version) as of the start of the called handler. For synchronous handlers, this
/// state will not be updated (externally) by other handlers. For asynchronous handlers, this state
/// may be updated by other handlers. Use the connection handle to queue closures that get the current
/// state.
pub struct Ctx<'a, S> {
    /// The per-connection state, as of the start of this handler.
    pub state: &'a S,

    /// The per-connection phase, as of the start of this handler.
    pub phase: Phase,

    /// The per-connection version, as of the start of this handler.
    pub version: ProtocolVersion,

    /// The connection handle. This is used to interact with the connection. Clone to use in async
    /// contexts. Actions are handled asynchronously. As such, they are applied after the synchronous
    /// part of the handler completes.
    pub handle: &'a ConnectionHandle<S>,
}

impl<'a, S> Ctx<'a, S> {
    pub(crate) fn new(
        state: &'a S,
        phase: Phase,
        version: ProtocolVersion,
        handle: &'a ConnectionHandle<S>,
    ) -> Self {
        Self {
            state,
            phase,
            version,
            handle,
        }
    }
}
