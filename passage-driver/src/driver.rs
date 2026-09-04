//! The driver: the only thing that owns the socket, the state, the phase and the version.
//!
//! The driver does four things and nothing else -- frame, dispatch, tick, and shut down. All
//! protocol logic lives in the [`Router`]'s handlers, and everything a handler wants to happen it
//! queues as an [`Op`].
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
//! `Op::Spawn` that preceded it has been counted.
//!
//! # The read gate
//!
//! [`ConnHandle::exclusive`](crate::conn::ConnHandle::exclusive) marks work the peer is expected to
//! wait for -- an authentication call, a session-server round trip. While such a task is in flight
//! the driver still **polls** the socket, and treats what it finds as a protocol break rather than
//! as input to be replayed later:
//!
//! * a frame is [`ProtocolError::EarlyPacket`] -- a compliant peer had nothing to send;
//! * an EOF ends the connection immediately, instead of after the adapter call returns on a
//!   connection nobody is on the other end of any more.
//!
//! Work that must genuinely overlap with further traffic uses
//! [`ConnHandle::spawn`](crate::conn::ConnHandle::spawn) instead. That is the difference between
//! "the framework decided to run my handlers concurrently" and "I asked for concurrency here".

use crate::codec::{Encoded, Frame, FrameCodec};
use crate::conn::{ConnHandle, Ctx, Op};
use crate::error::{ProtocolError, Result};
use crate::packet::Phase;
use crate::router::{Dispatched, Router, Table};
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Instant, Sleep, sleep_until};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};

/// How a connection ended.
///
/// Ending is not an error. Distinguishing *how* it ended is what lets the caller log a scanner
/// hang-up at `debug` and a decoding bug at `warn` without inspecting error variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// A handler asked to close the connection (status response sent, transfer sent, disconnect).
    Closed,

    /// The peer hung up.
    PeerClosed,

    /// The connection was cancelled by a shutdown.
    Cancelled,

    /// A deadline expired.
    TimedOut,
}

/// Static configuration of a connection.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    /// The decoding limits.
    pub limits: Limits,

    /// How often the tick handler runs, if at all. Ignored if the router has no tick handler.
    pub tick_interval: Option<Duration>,

    /// Hard cap on the whole connection.
    ///
    /// Passage connections are short by construction: a status ping is two packets and a login is a
    /// handful. This belongs to the driver rather than to the caller because the driver owns the
    /// clock and the socket -- wrapping [`Driver::run`] in [`tokio::time::timeout`] drops the
    /// future mid-flight, so the shutdown path never runs and in-flight tasks are not cancelled
    /// cleanly. It is also the backstop for a task that never resolves while the read gate is shut.
    pub max_lifetime: Option<Duration>,

    /// How long the peer may send nothing before the connection is dropped.
    pub max_idle: Option<Duration>,

    /// The protocol version before the handshake is processed.
    pub initial_version: ProtocolVersion,

    /// The phase the connection starts in.
    pub initial_phase: Phase,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            tick_interval: None,
            max_lifetime: None,
            max_idle: None,
            initial_version: ProtocolVersion::UNKNOWN,
            initial_phase: Phase::Handshake,
        }
    }
}

/// A handler task, paired with whether the peer has to stay quiet until it resolves.
type Task = BoxFuture<'static, (bool, Result<()>)>;

/// Drives one connection.
pub struct Driver<S, T> {
    framed: Framed<T, FrameCodec>,
    router: Arc<Router<S>>,
    table: Arc<Table>,
    state: S,
    handle: ConnHandle<S>,
    ops: mpsc::UnboundedReceiver<Op<S>>,
    tasks: FuturesUnordered<Task>,
    /// How many in-flight tasks require the peer to stay quiet. Maintained by the driver, so it
    /// cannot be left set by a handler that forgot to reset it.
    exclusive: usize,
    version: ProtocolVersion,
    phase: Phase,
    ticker: Option<tokio::time::Interval>,
    lifetime: Option<Pin<Box<Sleep>>>,
    idle: Option<Pin<Box<Sleep>>>,
    shutdown: CancellationToken,
    config: DriverConfig,
}

/// What the select in the main loop produced.
enum Step<S> {
    Op(Op<S>),
    Task(Option<(bool, Result<()>)>),
    Shutdown,
    Expired,
    Tick,
    Frame(Option<Result<Frame>>),
}

impl<S, T> Driver<S, T>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
{
    /// Creates a driver for `io`, together with the handle for its connection.
    ///
    /// This cannot fail: everything that could be misconfigured about a router was resolved by
    /// [`RouterBuilder::build`](crate::router::RouterBuilder::build) at startup.
    ///
    /// The handle is returned so the caller can talk to the connection from the outside -- close
    /// it, or hand it to something that will. Handlers get their own copy through [`Ctx`].
    pub fn new(
        io: T,
        router: Arc<Router<S>>,
        state: S,
        config: DriverConfig,
        shutdown: CancellationToken,
    ) -> (Self, ConnHandle<S>) {
        let table = router.table(config.initial_version);
        let (handle, ops) = ConnHandle::new(shutdown.clone());

        // A timer with no handler behind it would only wake the task up to do nothing.
        let ticker = config
            .tick_interval
            .filter(|_| router.ticks())
            .map(|interval| {
                let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker
            });

        let driver = Self {
            framed: Framed::new(io, FrameCodec::new(config.limits)),
            router,
            table,
            state,
            handle: handle.clone(),
            ops,
            tasks: FuturesUnordered::new(),
            exclusive: 0,
            version: config.initial_version,
            phase: config.initial_phase,
            ticker,
            lifetime: config
                .max_lifetime
                .map(|after| Box::pin(sleep_until(Instant::now() + after))),
            idle: config
                .max_idle
                .map(|after| Box::pin(sleep_until(Instant::now() + after))),
            shutdown,
            config,
        };
        (driver, handle)
    }

    /// Runs the connection to completion.
    ///
    /// Returns how it ended, or the error that ended it. Peer errors are returned like any other:
    /// the caller decides the log level from [`Error::class`](crate::error::Error::class).
    pub async fn run(mut self) -> Result<Completion> {
        let completion = loop {
            let step = tokio::select! {
                biased;

                // 1. Everything handlers asked for, before anything else.
                op = self.ops.recv() => Step::Op(
                    op.expect("the driver holds a handle, so the queue cannot close"),
                ),

                // 2. Finished handler tasks, exclusive or not -- one set, one arm.
                done = self.tasks.next(), if !self.tasks.is_empty() => Step::Task(done),

                // 3. Cancellation.
                () = self.shutdown.cancelled() => Step::Shutdown,

                // 4. Deadlines.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => Step::Expired,
                () = expire(&mut self.idle), if self.idle.is_some() => Step::Expired,

                // 5. Ticks -- but never while the peer must stay quiet. A keep-alive sent into a
                //    gated window would invite the very packet the gate rejects.
                () = tick(&mut self.ticker),
                    if self.ticker.is_some() && self.exclusive == 0 => Step::Tick,

                // 6. Input. Polled even while gated: see the module docs.
                frame = self.framed.next() => Step::Frame(frame),
            };

            match step {
                Step::Op(op) => {
                    if let Some(completion) = self.handle_op(op).await? {
                        break completion;
                    }
                    // One write for a batch of packets rather than one per packet.
                    if self.ops.is_empty() {
                        self.flush().await?;
                    }
                }
                // The set was drained between the guard and the poll; nothing to do.
                Step::Task(None) => {}
                Step::Task(Some((exclusive, result))) => {
                    if exclusive {
                        self.exclusive = self.exclusive.saturating_sub(1);
                    }
                    result?;
                }
                Step::Shutdown => break Completion::Cancelled,
                Step::Expired => break Completion::TimedOut,
                Step::Tick => self.handle_tick()?,
                Step::Frame(None) => break Completion::PeerClosed,
                Step::Frame(Some(frame)) => self.handle_frame(frame?)?,
            }
        };

        self.finish().await;
        Ok(completion)
    }

    /// Carries out one queued operation. Returns a completion if the connection should end.
    async fn handle_op(&mut self, op: Op<S>) -> Result<Option<Completion>> {
        match op {
            Op::Send(packet) => {
                // Encoded here, at drain time, so the bytes are ordered by the queue rather than by
                // whenever a handler happened to finish building the packet -- and so a handler
                // never needs to know the version to send something.
                let encoded = Encoded::of_any(&*packet, self.version, self.config.limits)?;
                trace!(packet = encoded.name, "writing packet");
                self.framed.feed(encoded).await?;
            }
            Op::Encrypt(cipher) => {
                debug!("enabling encryption");
                // The codec only encrypts what it encodes from here on, so bytes already buffered
                // stay plaintext and no flush is needed to get the switchover point right.
                self.framed.codec_mut().set_cipher(cipher);
            }
            Op::SetVersion(version) => {
                debug!(%version, "binding dispatch table");
                self.version = version;
                self.table = self.router.table(version);
            }
            Op::SetPhase(phase) => {
                trace!(?phase, "entering phase");
                self.phase = phase;
            }
            Op::With(change) => change(&mut self.state),
            Op::Spawn { future, exclusive } => {
                if exclusive {
                    self.exclusive += 1;
                }
                self.tasks
                    .push(Box::pin(async move { (exclusive, future.await) }));
            }
            Op::Flush(waiter) => {
                self.flush().await?;
                // The waiter having gone away is fine: it only means nobody is listening anymore.
                let _ = waiter.send(());
            }
            Op::Close => {
                self.flush().await?;
                return Ok(Some(Completion::Closed));
            }
        }
        Ok(None)
    }

    fn handle_tick(&mut self) -> Result<()> {
        let Some(handler) = self.router.tick_handler() else {
            return Ok(());
        };
        handler.call(Ctx::new(
            &self.state,
            &self.handle,
            self.config.limits,
            self.version,
            self.phase,
        ))
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<()> {
        // Anything the peer sends counts as liveness, including what is rejected below.
        self.reset_idle();

        if self.exclusive > 0 {
            return Err(ProtocolError::EarlyPacket {
                phase: self.phase,
                id: frame.id,
            }
            .into());
        }

        let ctx = Ctx::new(
            &self.state,
            &self.handle,
            self.config.limits,
            self.version,
            self.phase,
        );
        match self
            .router
            .dispatch(&self.table, ctx, frame.id, &frame.payload)?
        {
            Dispatched::Handled(name) => {
                trace!(packet = name, phase = ?self.phase, "dispatched packet");
            }
            Dispatched::Ignored => {
                trace!(id = frame.id, phase = ?self.phase, "ignoring unhandled packet");
            }
        }
        Ok(())
    }

    fn reset_idle(&mut self) {
        if let (Some(idle), Some(after)) = (self.idle.as_mut(), self.config.max_idle) {
            idle.as_mut().reset(Instant::now() + after);
        }
    }

    /// Writes whatever has been buffered, if anything.
    async fn flush(&mut self) -> Result<()> {
        if self.framed.write_buffer().is_empty() {
            return Ok(());
        }
        self.framed.flush().await
    }

    /// Ends the connection: stop everything we started, then let the socket go.
    async fn finish(mut self) {
        self.shutdown.cancel();
        // The tasks are polled on this task, so dropping them *is* cancellation.
        self.tasks.clear();
        if let Err(err) = self.framed.flush().await {
            debug!(cause = %err, "failed to flush on close");
        }
        if let Err(err) = self.framed.close().await {
            debug!(cause = %err, "failed to close the socket");
        }
    }
}

/// Awaits a deadline, or never if there is none.
async fn expire(sleep: &mut Option<Pin<Box<Sleep>>>) {
    match sleep {
        Some(sleep) => sleep.as_mut().await,
        None => std::future::pending().await,
    }
}

/// Awaits the next tick, or never if there is no ticker.
async fn tick(ticker: &mut Option<tokio::time::Interval>) {
    match ticker {
        Some(ticker) => {
            ticker.tick().await;
        }
        None => std::future::pending().await,
    }
}
