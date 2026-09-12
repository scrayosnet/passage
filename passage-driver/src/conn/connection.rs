//! The connection loop.
//!
//! See the [module docs](crate::conn) for the vocabulary, the priority order of the loop, the read
//! gate and what happens when a connection ends.

use crate::codec::{Encoded, Frame, FrameCodec};
use crate::conn::{ConnectionHandle, Ctx, Dispatcher, Op};
use crate::error::{Error, InternalError, ProtocolError, Result};
use crate::packet::Phase;
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt};
use std::ops::ControlFlow;
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Instant, Sleep, sleep_until};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};

/// How long a connection may spend writing what it owes the peer once it is already ending.
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a connection may last by default.
///
/// A default is the security posture, and "no bound at all" is not one: the documented minimal
/// `serve(listener, router, state).await` would otherwise accept sockets that idle forever, and
/// holding one open costs a peer nothing. Two minutes is what the previous implementation's
/// listener applied and is an eternity for a handshake, a login and a transfer. A server that
/// genuinely runs long connections -- anything that reaches [`Phase::Play`] -- says so by setting
/// this itself.
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(120);

/// What ended a connection that we did not end ourselves.
///
/// The `Err` half of [`Outcome::result`], and exactly what
/// [`Dispatcher::on_error`](crate::conn::Dispatcher::on_error) is called for -- the two are the same
/// set, which is why the trait can take this and nothing else.
///
/// The line it draws is **who decided**. `Ok(())` means a handler asked to close, and there is
/// nothing left to do or say. Everything else is here, including a peer that simply hung up: the
/// point is not that a hangup is a *failure* -- it is not, and [`error`](Ending::error) says so --
/// but that we did not finish what we were doing. Whatever a connection reserved, counted or
/// promised on the way in needs releasing on the way out, and that has to happen for a client that
/// vanished mid-login exactly as it does for one that timed out.
#[derive(Debug, thiserror::Error)]
pub enum Ending {
    /// The peer hung up.
    ///
    /// Nothing can be sent after this. It is still an ending rather than a completion because the
    /// peer leaving is not us being finished with it -- a client that disappears while its backend
    /// is being selected has left a selection running.
    #[error("the peer hung up")]
    PeerClosed,

    /// The shutdown token was cancelled.
    #[error("the connection was cancelled")]
    Cancelled,

    /// A deadline expired.
    #[error("the connection ran out of time")]
    TimedOut,

    /// A handler, a decode, a background task or the transport failed.
    #[error(transparent)]
    Failed(#[from] Error),
}

impl Ending {
    /// The error that ended the connection, if anything failed at all.
    ///
    /// `None` for a hangup, a cancellation or a deadline. Those are things that *happened*, not
    /// things that went wrong, and nobody is to blame for them -- which is also why there is no
    /// `class()` here. Blame is a property of an [`Error`], and three of these four have none.
    #[must_use]
    pub fn error(&self) -> Option<&Error> {
        match self {
            Ending::Failed(err) => Some(err),
            Ending::PeerClosed | Ending::Cancelled | Ending::TimedOut => None,
        }
    }

    /// Whether anything can still be written to the peer.
    ///
    /// Only a hangup answers this for certain. A broken transport or a peer that stopped reading
    /// will refuse the write too, but there is no way to know that without trying -- so this is the
    /// one case worth checking before composing a message nobody will read.
    #[must_use]
    pub fn can_reply(&self) -> bool {
        !matches!(self, Ending::PeerClosed)
    }

    /// A stable, low-cardinality label for metrics. Never contains peer-controlled data.
    ///
    /// Defined for every ending, not just the failures, because "how did connections end" is one
    /// question and wants one label.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Ending::PeerClosed => "peer_closed",
            Ending::Cancelled => "cancelled",
            Ending::TimedOut => "timed_out",
            Ending::Failed(err) => err.label(),
        }
    }
}

/// Everything a connection knows about itself when it ends.
///
/// The state comes back by value because it is the connection's -- nothing else ever held it -- and
/// because a caller driving a connection directly may want what it accumulated: the hostname that
/// was asked for, the profile that was verified, the intent from the handshake.
///
/// [`Server`](crate::server::Server) drops it. That is not an oversight: a fact worth recording is
/// better recorded by the handler that established it, while it still has the packet and the phase
/// in hand, than by something downstream reading it back out of a struct.
#[derive(Debug)]
pub struct Outcome<S> {
    /// How it ended: `Ok` if a handler closed it, `Err` for every other way.
    ///
    /// There is nothing in the `Ok` half because there is nothing to say -- the handler that closed
    /// knows why, and wrote whatever mattered into [`state`](Outcome::state).
    pub result: Result<(), Ending>,

    /// The connection state, as it was left.
    pub state: S,

    /// The protocol version it settled on.
    pub version: ProtocolVersion,

    /// The phase it reached.
    pub phase: Phase,
}

/// Static configuration of a connection.
///
/// Plain data, and `Copy`: every field is, and a connection takes its own copy rather than sharing
/// the server's.
#[derive(Copy, Clone, Debug)]
pub struct ConnectionConfig {
    /// The decoding limits.
    pub limits: Limits,

    /// How often the tick handler runs, if at all. Ignored if
    /// [`Dispatcher::ticks`](crate::conn::Dispatcher::ticks) is `false`.
    ///
    /// This is also where a read deadline belongs: a tick handler sees the phase and the state, so
    /// "nothing has arrived and we are still waiting for the handshake" is a check it can make and
    /// the connection cannot.
    pub tick_interval: Option<Duration>,

    /// Hard cap on the whole connection. Two minutes unless you say otherwise.
    ///
    /// Passage connections are short by construction: a status ping is two packets and a login is a
    /// handful. This belongs to the connection rather than to the caller because the connection owns the
    /// clock and the socket -- wrapping [`Connection::run`] in [`tokio::time::timeout`] drops the
    /// future mid-flight, so the shutdown path never runs and in-flight tasks are not cancelled
    /// cleanly. It is also the backstop for a task that never resolves while the read gate is shut,
    /// and for a peer that stops reading (every socket write is raced against it).
    ///
    /// `None` removes the cap, which is what a server that reaches [`Phase::Play`] wants -- and is
    /// a deliberate statement rather than the default, because a socket nobody bounds is free for a
    /// peer to hold and not for us.
    pub max_lifetime: Option<Duration>,

    /// How long the connection may spend writing what it owes the peer *after* it has started
    /// ending.
    ///
    /// The final flush cannot be bounded by [`max_lifetime`](ConnectionConfig::max_lifetime) or by
    /// the shutdown token, because an expired deadline and a cancelled token are two of the reasons
    /// there is a disconnect message to write in the first place. Without this bound, a peer that
    /// stops reading would keep the connection -- and any graceful shutdown waiting on it -- alive
    /// forever.
    pub close_timeout: Option<Duration>,

    /// The protocol version before the handshake is processed.
    pub initial_version: ProtocolVersion,

    /// The phase the connection starts in.
    pub initial_phase: Phase,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            tick_interval: None,
            max_lifetime: Some(DEFAULT_MAX_LIFETIME),
            close_timeout: Some(DEFAULT_CLOSE_TIMEOUT),
            initial_version: ProtocolVersion::UNKNOWN,
            initial_phase: Phase::Handshake,
        }
    }
}

/// A handler task, paired with whether the peer has to stay quiet until it resolves.
type Task = BoxFuture<'static, (bool, Result<()>)>;

/// Collects what a [`Connection`] is created with. See [`Connection::builder`].
pub struct ConnectionBuilder<S, T, D> {
    io: T,
    dispatcher: D,
    state: S,
    config: ConnectionConfig,
    shutdown: Option<CancellationToken>,
}

impl<S, T, D> ConnectionBuilder<S, T, D>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
    D: Dispatcher<S>,
{
    /// Sets the configuration. Defaults to [`ConnectionConfig::default`].
    #[must_use]
    pub fn config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Cancels the connection when `shutdown` is cancelled.
    ///
    /// Defaults to a token of the connection's own, which it cancels when it ends -- so leaving
    /// this unset means "nothing outside can stop it early", not "it can never be stopped".
    #[must_use]
    pub fn shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Creates the connection, together with the handle for talking to it from outside.
    ///
    /// This cannot fail. Everything that could be misconfigured about dispatch was resolved when the
    /// dispatcher was built.
    pub fn build(self) -> (Connection<S, T, D>, ConnectionHandle<S>) {
        Connection::new(
            self.io,
            self.dispatcher,
            self.state,
            self.config,
            self.shutdown.unwrap_or_default(),
        )
    }
}

/// Drives one connection.
pub struct Connection<S, T, D> {
    framed: Framed<T, FrameCodec>,
    /// Where frames, ticks and endings go. The connection holds no dispatch table of its own, and
    /// no reference to a router -- see [`Dispatcher`].
    dispatcher: D,
    state: S,
    handle: ConnectionHandle<S>,
    ops: mpsc::UnboundedReceiver<Op<S>>,
    tasks: FuturesUnordered<Task>,
    /// How many in-flight tasks require the peer to stay quiet. Maintained by the connection, so it
    /// cannot be left set by a handler that forgot to reset it.
    exclusive: usize,
    version: ProtocolVersion,
    phase: Phase,
    ticker: Option<tokio::time::Interval>,
    lifetime: Option<Pin<Box<Sleep>>>,
    /// Armed once the connection starts ending; bounds everything written from then on.
    closing: Option<Pin<Box<Sleep>>>,
    shutdown: CancellationToken,
    config: ConnectionConfig,
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

impl<S, T, D> Connection<S, T, D>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
    D: Dispatcher<S>,
{
    /// Starts building a connection over `io`.
    ///
    /// The three arguments are the ones a connection cannot be created without, and they are of
    /// three unmistakably different kinds. Everything else -- the configuration, the shutdown token
    /// -- has a default and is set by name, which is what the five positional arguments this
    /// replaces could not offer: `dispatcher`, `state` and `config` all looked like "some
    /// `S`-shaped thing" at the call site.
    pub fn builder(io: T, dispatcher: D, state: S) -> ConnectionBuilder<S, T, D> {
        ConnectionBuilder {
            io,
            dispatcher,
            state,
            config: ConnectionConfig::default(),
            shutdown: None,
        }
    }

    /// Creates the connection, together with the handle for it.
    ///
    /// This cannot fail. Everything that could be misconfigured about dispatch was resolved when
    /// the dispatcher was built -- for the built-in one, by
    /// [`RouterBuilder::build`](crate::router::RouterBuilder::build) at startup.
    ///
    /// The handle is returned so the caller can talk to the connection from the outside -- close
    /// it, or hand it to something that will. Handlers get their own copy through [`Ctx`].
    fn new(
        io: T,
        mut dispatcher: D,
        state: S,
        config: ConnectionConfig,
        shutdown: CancellationToken,
    ) -> (Self, ConnectionHandle<S>) {
        // The dispatcher is handed over unbound, so the connection is the only thing that decides
        // which version dispatch happens against -- here and in `Op::SetVersion`, nowhere else.
        dispatcher.set_version(config.initial_version);

        let (handle, ops) =
            ConnectionHandle::new(shutdown.clone(), config.initial_version, config.limits);

        // A timer with no handler behind it would only wake the task up to do nothing.
        let ticker = config
            .tick_interval
            .filter(|_| dispatcher.ticks())
            .map(|interval| {
                let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker
            });

        let connection = Self {
            framed: Framed::new(io, FrameCodec::new(config.limits)),
            dispatcher,
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
            closing: None,
            shutdown,
            config,
        };
        (connection, handle)
    }

    /// Runs the connection to completion.
    ///
    /// Peer errors come back like any other: the caller decides the log level from
    /// [`Error::class`](crate::error::Error::class).
    pub async fn run(mut self) -> Outcome<S> {
        let result = self.serve().await;
        self.finish().await;
        Outcome {
            result,
            state: self.state,
            version: self.version,
            phase: self.phase,
        }
    }

    /// Runs the loop, and gives the dispatcher the last word on anything it did not ask for.
    async fn serve(&mut self) -> Result<(), Ending> {
        let Err(ending) = self.drive().await else {
            // A handler closed it, so there is nothing left to say.
            return Ok(());
        };

        // Everything from here on is written under the closing deadline instead of the shutdown
        // token -- a cancelled token is one of the reasons there is something to say.
        self.arm_closing();

        // Whatever the failing handler queued before it failed is still written: "send this
        // disconnect, then fail" has to mean what it says.
        self.settle().await;

        let ctx = Ctx::new(&self.state, &self.handle, self.phase);
        if let Err(err) = self.dispatcher.on_error(ctx, &ending) {
            // Logged, not reported. The connection is ending for the reason below; that the
            // apology would not encode is a detail of the answer, not a second cause.
            debug!(cause = %err, "the dispatcher could not answer the ending");
        }
        self.settle().await;

        Err(ending)
    }

    /// The main loop: frames in, operations out, until something ends it.
    async fn drive(&mut self) -> Result<(), Ending> {
        loop {
            let step = tokio::select! {
                biased;

                // 1. Everything handlers asked for, before anything else.
                op = self.ops.recv() => Step::Op(
                    op.expect("the connection holds a handle, so the queue cannot close"),
                ),

                // 2. Finished handler tasks, exclusive or not -- one set, one arm.
                done = self.tasks.next(), if !self.tasks.is_empty() => Step::Task(done),

                // 3. Cancellation.
                () = self.shutdown.cancelled() => Step::Shutdown,

                // 4. The deadline.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => Step::Expired,

                // 5. Ticks -- but never while the peer must stay quiet. A keep-alive sent into a
                //    gated window would invite the very packet the gate rejects.
                () = tick(&mut self.ticker),
                    if self.ticker.is_some() && self.exclusive == 0 => Step::Tick,

                // 6. Input. Polled even while gated: see the module docs.
                frame = self.framed.next() => Step::Frame(frame),
            };

            match step {
                Step::Op(op) => {
                    if self.handle_op(op).await?.is_break() {
                        return Ok(());
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
                Step::Shutdown => return Err(Ending::Cancelled),
                Step::Expired => return Err(Ending::TimedOut),
                Step::Tick => self.handle_tick()?,
                Step::Frame(None) => return Err(Ending::PeerClosed),
                Step::Frame(Some(frame)) => self.handle_frame(frame?)?,
            }
        }
    }

    /// Carries out one queued operation, and says whether it ended the connection.
    async fn handle_op(&mut self, op: Op<S>) -> Result<ControlFlow<()>, Ending> {
        match op {
            Op::Send { encoded, version } => {
                // The bytes were encoded against a snapshot of the version. Refuse them if the
                // connection has moved on since, rather than writing an ID the peer resolves in
                // another table.
                if version != self.version {
                    return Err(Error::from(InternalError::StaleEncoding {
                        packet: encoded.name,
                        encoded_version: version,
                        version: self.version,
                    })
                    .into());
                }
                trace!(packet = encoded.name, "writing packet");
                self.write(encoded).await?;
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
                self.dispatcher.set_version(version);
                // Handlers encode against the handle's version, so it has to move with us. Ours is
                // the one every `Ctx` lends out, which is why a handle taken from a handler is
                // always current.
                self.handle = self.handle.at_version(version);
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
            // Boxed because an operation may hold operations. Only batches pay for it.
            Op::Batch(ops) => {
                for op in ops {
                    if Box::pin(self.handle_op(op)).await?.is_break() {
                        return Ok(ControlFlow::Break(()));
                    }
                }
            }
            Op::Fail(err) => return Err(err.into()),
            Op::Close => {
                self.flush().await?;
                return Ok(ControlFlow::Break(()));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn handle_tick(&mut self) -> Result<()> {
        self.dispatcher
            .tick(Ctx::new(&self.state, &self.handle, self.phase))
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<()> {
        if self.exclusive > 0 {
            return Err(ProtocolError::EarlyPacket {
                phase: self.phase,
                id: frame.id,
            }
            .into());
        }

        let ctx = Ctx::new(&self.state, &self.handle, self.phase);
        self.dispatcher.dispatch(ctx, frame.id, &frame.payload)
    }

    /// Buffers a packet, under whichever deadline currently applies.
    ///
    /// The socket has to be written *somewhere*, and every candidate has the same problem: a peer
    /// that stops reading fills the write buffer and the write stops making progress. Doing it
    /// inside the loop's `select!` would mean re-entering a half-finished write on every wakeup,
    /// so it happens here instead -- with an escape, so a peer that will not read cannot outlast
    /// its deadline or ignore a shutdown.
    async fn write(&mut self, encoded: Encoded) -> Result<(), Ending> {
        let write = self.framed.feed(encoded);
        guarded(&self.shutdown, &mut self.lifetime, &mut self.closing, write).await
    }

    /// Writes whatever has been buffered, if anything, under the same guard as [`write`].
    async fn flush(&mut self) -> Result<(), Ending> {
        if self.framed.write_buffer().is_empty() {
            return Ok(());
        }
        let flush = self.framed.flush();
        guarded(&self.shutdown, &mut self.lifetime, &mut self.closing, flush).await
    }

    /// Switches the write guard over to the closing deadline.
    ///
    /// `closing` being armed is exactly the statement "this connection is ending", and every write
    /// after it is bounded by that one clock instead of by the token and the lifetime -- both of
    /// which may be the very reason it is ending.
    fn arm_closing(&mut self) {
        if self.closing.is_none()
            && let Some(after) = self.config.close_timeout
        {
            self.closing = Some(Box::pin(sleep_until(Instant::now() + after)));
        }
    }

    /// Carries out everything still queued, for a connection that is already ending.
    ///
    /// The same [`handle_op`](Self::handle_op) as the loop uses -- an ending is not a different
    /// vocabulary, only a different clock, and [`arm_closing`](Self::arm_closing) has already
    /// switched that over. What differs is what a failure means: the reason the connection is
    /// ending has been decided, and losing it to a broken socket on the way out would replace the
    /// diagnosis with the symptom, so failures here are logged and the drain stops.
    async fn settle(&mut self) {
        while let Ok(op) = self.ops.try_recv() {
            match self.handle_op(op).await {
                // A handler had already asked to end; there is nothing after it to write.
                Ok(ControlFlow::Break(())) => return,
                Ok(ControlFlow::Continue(())) => {}
                Err(ending) => {
                    debug!(?ending, "gave up on what was still queued");
                    return;
                }
            }
        }
        if let Err(ending) = self.flush().await {
            debug!(?ending, "gave up flushing what was still queued");
        }
    }

    /// Ends the connection: stop everything we started, then let the socket go.
    async fn finish(&mut self) {
        // The tasks are polled on this task, so dropping them *is* cancellation.
        self.tasks.clear();

        self.arm_closing();
        if let Err(ending) = self.flush().await {
            debug!(?ending, "failed to flush on close");
        }
        let close = self.framed.close();
        if let Err(ending) =
            guarded(&self.shutdown, &mut self.lifetime, &mut self.closing, close).await
        {
            debug!(?ending, "failed to close the socket");
        }

        // Last, so that anything still watching the token sees it only once there is nothing left
        // to write. Cancelling first would race the flush above against every detached task.
        self.shutdown.cancel();
    }
}

/// Awaits a socket operation under whichever deadline applies.
///
/// While the connection is running that is the shutdown token and the lifetime. Once `closing` is
/// armed the connection is already ending, and that one clock takes over -- because a cancelled
/// token and an expired deadline are two of the three reasons there is a last packet to write.
async fn guarded<T>(
    shutdown: &CancellationToken,
    lifetime: &mut Option<Pin<Box<Sleep>>>,
    closing: &mut Option<Pin<Box<Sleep>>>,
    future: impl Future<Output = Result<T>>,
) -> Result<T, Ending> {
    if closing.is_some() {
        return tokio::select! {
            biased;

            result = future => Ok(result?),
            () = expire(closing) => Err(Ending::TimedOut),
        };
    }

    tokio::select! {
        biased;

        result = future => Ok(result?),
        () = shutdown.cancelled() => Err(Ending::Cancelled),
        () = expire(lifetime), if lifetime.is_some() => Err(Ending::TimedOut),
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
