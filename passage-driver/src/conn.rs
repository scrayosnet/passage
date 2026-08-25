//! The connection handle, the operation queue and the handler context.
//!
//! Handlers never touch the socket. They queue [`Op`]s on an ordered channel that the driver drains
//! with priority. Two properties fall out of that:
//!
//! * **One writer.** The driver is the only thing that writes to the socket, so no lock is needed
//!   and no interleaving is possible -- even when a handler queues packets from a detached task.
//! * **Ordered side effects.** Enabling encryption is an operation like any other, so it lands in
//!   the stream exactly between the packet queued before it and the one queued after. Getting this
//!   wrong is the classic "works until the client is slow" bug.

use crate::codec::{Cipher, Encoded};
use crate::error::{Error, Result};
use crate::flow::Update;
use crate::packet::{Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU8, Ordering};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// An operation for the driver to carry out, in queue order.
pub enum Op<S> {
    /// Write a packet to the socket.
    Send(Encoded),

    /// Enable encryption for every byte from here on.
    Encrypt(Box<dyn Cipher>),

    /// Run a future alongside the connection, cancelling it when the connection ends.
    Detach(BoxFuture<'static, Result<Update<S>>>),

    /// Flush the socket and notify the waiter once everything queued before has been written.
    Flush(oneshot::Sender<()>),

    /// Finish the connection after writing everything queued before.
    Close,
}

impl<S> std::fmt::Debug for Op<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Op::Send(encoded) => write!(f, "Send({})", encoded.name),
            Op::Encrypt(_) => f.write_str("Encrypt"),
            Op::Detach(_) => f.write_str("Detach"),
            Op::Flush(_) => f.write_str("Flush"),
            Op::Close => f.write_str("Close"),
        }
    }
}

struct Shared<S> {
    version: AtomicI32,
    phase: AtomicU8,
    ops: mpsc::UnboundedSender<Op<S>>,
    shutdown: CancellationToken,
}

/// A cheap, cloneable handle to a live connection.
///
/// This is what a handler keeps when it moves work into a detached task. Everything on it is either
/// an atomic load or a channel send, so it is safe to hold across await points.
pub struct ConnHandle<S> {
    shared: Arc<Shared<S>>,
}

// Derived `Clone` would demand `S: Clone`, which is wrong: the state is never cloned, only shared.
impl<S> Clone for ConnHandle<S> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<S> ConnHandle<S> {
    /// Creates a handle and the receiving end of its operation queue.
    pub(crate) fn new(
        version: ProtocolVersion,
        phase: Phase,
        shutdown: CancellationToken,
    ) -> (Self, mpsc::UnboundedReceiver<Op<S>>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let shared = Shared {
            version: AtomicI32::new(version.get()),
            phase: AtomicU8::new(phase.index() as u8),
            ops: tx,
            shutdown,
        };
        (
            Self {
                shared: Arc::new(shared),
            },
            rx,
        )
    }

    /// The negotiated protocol version.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        ProtocolVersion::new(self.shared.version.load(Ordering::Acquire))
    }

    /// The current protocol phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        let index = self.shared.phase.load(Ordering::Acquire) as usize;
        Phase::from_index(index).unwrap_or(Phase::Handshake)
    }

    /// Sets the protocol version, taking effect immediately.
    ///
    /// This is the one thing a handler may change eagerly: it decides which IDs are used for
    /// packets sent from here on and which decode table the driver binds for the next frame. It is
    /// only ever called once, from the handshake handler, before any packet has been queued.
    pub fn set_version(&self, version: ProtocolVersion) {
        self.shared.version.store(version.get(), Ordering::Release);
    }

    /// Moves the connection to another phase, taking effect immediately.
    ///
    /// # Invariant
    ///
    /// A phase change must be triggered by the peer's transition packet (`intention`,
    /// `login acknowledged`, ...) or be preceded by [`ConnHandle::flush`]. Switching phases while
    /// the peer may still be sending packets of the old phase means decoding them against the wrong
    /// ID table.
    pub fn set_phase(&self, phase: Phase) {
        self.shared
            .phase
            .store(phase.index() as u8, Ordering::Release);
    }

    /// Encodes a packet and queues it for writing.
    ///
    /// The packet ID is resolved from the connection's protocol version here -- the single place in
    /// the whole stack where a packet ID is looked up for writing. Sending a packet that does not
    /// exist in the peer's version is an internal error, not a silent no-op.
    pub fn send<P: Packet>(&self, packet: &P) -> Result<()> {
        self.queue(Op::Send(Encoded::of(packet, self.version())?))
    }

    /// Enables encryption for every byte written or read after the operations queued so far.
    pub fn encrypt(&self, cipher: Box<dyn Cipher>) -> Result<()> {
        self.queue(Op::Encrypt(cipher))
    }

    /// Runs a future for as long as the connection lives.
    ///
    /// Use this for work that must overlap with further protocol traffic -- Passage's backend
    /// selection, which runs while keep-alives are exchanged. Anything the future needs from the
    /// connection it takes through a cloned [`ConnHandle`]; what it writes back it returns as an
    /// [`Update`].
    pub fn detach(
        &self,
        future: impl Future<Output = Result<Update<S>>> + Send + 'static,
    ) -> Result<()> {
        self.queue(Op::Detach(Box::pin(future)))
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
        &self.shared.shutdown
    }

    fn queue(&self, op: Op<S>) -> Result<()> {
        self.shared.ops.send(op).map_err(|_| Error::Closed)
    }
}

/// What a handler is given: mutable connection state plus a handle to the connection.
///
/// The state is a plain `&mut S`, not a lock. Synchronous handlers -- the vast majority -- get
/// exclusive access with no atomics and no contention. Asynchronous handlers cannot hold it across
/// an await point (the future in [`Flow::Pending`](crate::flow::Flow::Pending) is `'static`), which
/// removes the whole class of "lock held while calling an adapter" stalls.
pub struct Ctx<'a, S> {
    /// The per-connection state.
    pub state: &'a mut S,

    /// The connection handle.
    pub conn: &'a ConnHandle<S>,

    limits: Limits,
}

impl<'a, S> Ctx<'a, S> {
    pub(crate) fn new(state: &'a mut S, conn: &'a ConnHandle<S>, limits: Limits) -> Self {
        Self {
            state,
            conn,
            limits,
        }
    }

    /// The decoding limits of this connection.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The negotiated protocol version.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.conn.version()
    }

    /// The current protocol phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.conn.phase()
    }

    /// Encodes a packet and queues it for writing. See [`ConnHandle::send`].
    pub fn send<P: Packet>(&self, packet: &P) -> Result<()> {
        self.conn.send(packet)
    }
}
