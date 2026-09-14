use crate::codec::{CodecError, Frame, FrameCodec};
use crate::connection::{ConnectionHandle, Ctx, Dispatcher, Op, ConnectionError, Result};
use crate::packet::Phase;
use crate::version::ProtocolVersion;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt, TryFutureExt};
use std::ops::ControlFlow;
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::{Instant, Sleep, sleep_until};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};
use crate::wire::Options;

/// The default timeout for the `on_error` dispatcher hook.
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
    /// The connection's result error.
    pub error: Option<ConnectionError>,

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
    /// The decoding options.
    pub options: Options,

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
            options: Options::default(),
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
    pub fn build(self) -> Connection<S, T, D> {
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
    /// The peer socket connection, wrapped by a framed codec.
    framed: Framed<T, FrameCodec>,

    /// The dispatcher that handles the incoming frames, ticks, and errors.
    dispatcher: D,

    /// The custom connection scoped state.
    state: S,

    /// The list of tasks that have been queued by the dispatcher.
    tasks: FuturesUnordered<Task>,

    /// The number of tasks that are running exclusively. No new frames and ticks are handled while
    /// at least one task is exclusive.
    exclusive: usize,

    /// The current protocol version.
    version: ProtocolVersion,

    /// The current phase.
    phase: Phase,

    /// The timer for ticks.
    ticker: Option<tokio::time::Interval>,

    /// The timer for the connection's lifetime.
    lifetime: Option<Pin<Box<Sleep>>>,

    /// The connection's shutdown token.
    shutdown: CancellationToken,

    /// The connection's configuration.
    config: ConnectionConfig,
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
        dispatcher: D,
        state: S,
        config: ConnectionConfig,
        shutdown: CancellationToken,
    ) -> Self {
        // A timer with no handler behind it would only wake the task up to do nothing.
        let ticker = config
            .tick_interval
            .map(|interval| {
                let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                ticker
            });

        Self {
            framed: Framed::new(io, FrameCodec::new(config.options)),
            dispatcher,
            state,
            tasks: FuturesUnordered::new(),
            exclusive: 0,
            version: config.initial_version,
            phase: config.initial_phase,
            ticker,
            lifetime: config
                .max_lifetime
                .map(|after| Box::pin(sleep_until(Instant::now() + after))),
            shutdown,
            config,
        }
    }

    /// Runs the connection to completion.
    ///
    /// Peer errors come back like any other: the caller decides the log level from
    /// [`Error::class`](crate::error::Error::class).
    pub async fn run(mut self) -> Outcome<S> {
        let result = self.serve().await;

        // Close the socket.
        let close = self.framed.close().map_err(Into::into);
        if let Err(error) = guarded(&self.shutdown, &mut self.lifetime, close).await {
            debug!(%error, "failed to close the socket");
        }

        Outcome {
            error: result.err(),
            state: self.state,
            version: self.version,
            phase: self.phase,
        }
    }

    /// Runs the loop, and gives the dispatcher the last word on anything it did not ask for.
    async fn serve(&mut self) -> Result<()> {
        // Run the handlers until they complete or raise an error.
        let (handle, mut ops) = ConnectionHandle::new(self.shutdown.clone(), self.config.options);
        let Err(mut error) = self.drive(&handle, &mut ops).await else {
            return Ok(());
        };

        // Notify any detached tasks and re-create timeout limits for error hooks.
        self.tasks.clear();
        self.shutdown.cancel();

        // Restart the drive with the error handler.
        self.shutdown = CancellationToken::new();
        self.lifetime = Some(Box::pin(sleep_until(Instant::now() + DEFAULT_CLOSE_TIMEOUT)));
        let (handle, mut ops) = ConnectionHandle::new(self.shutdown.clone(), self.config.options);

        // Run the error hook and settle its results.
        let ctx = Ctx::new(&self.state, self.phase, self.version, &handle);
        if let Err(error) = self.dispatcher.on_error(ctx, &mut error) {
            debug!(%error, "failed to complete error handling");
        }
        if let Err(error) = self.drive(&handle, &mut ops).await {
            debug!(%error, "failed to complete error handling");
        };

        // Notify any detached task of the error hook to complete.
        self.tasks.clear();
        self.shutdown.cancel();

        Err(error)
    }

    /// The main loop: frames in, operations out, until something ends it.
    async fn drive(&mut self, handle: &ConnectionHandle<S>, ops: &mut mpsc::UnboundedReceiver<Op<S>>) -> Result<()> {
        loop {
            tokio::select! {
                biased;

                // 1. Everything handlers asked for, before anything else.
                op = ops.recv() => {
                    // TODO handle errors better?
                    let flow = self.handle_op(handle, op.expect("cannot be closed")).await?;
                    if ops.is_empty() {
                        self.flush().await?;
                    }
                    if flow.is_break() {
                        return Ok(());
                    };
                },

                // 2. Finished handler tasks.
                done = self.tasks.next(), if !self.tasks.is_empty() => {
                    let (exclusive, result) = done.expect("cannot be empty");
                    self.exclusive = self.exclusive.saturating_sub(exclusive.then(|| 1).unwrap_or(0));
                    result?;
                },

                // 3. Cancellation.
                () = self.shutdown.cancelled() => {
                    return Err(ConnectionError::Cancelled);
                },

                // 4. The deadline.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => {
                    return Err(ConnectionError::TimedOut);
                },

                // 5. Ticks -- but never while the peer must stay quiet. A keep-alive sent into a
                //    gated window would invite the very packet the gate rejects.
                () = tick(&mut self.ticker), if self.exclusive == 0 => {
                    self.handle_tick(handle)?;
                },

                // 6. Input. Polled even while gated: see the module docs.
                frame = self.framed.next() => {
                    let Some(frame) = frame else {
                        return Err(ConnectionError::PeerClosed);
                    };
                    self.handle_frame(handle, frame?)?;
                },
            };
        }
    }

    /// Carries out one queued operation, and says whether it ended the connection.
    async fn handle_op(&mut self, handle: &ConnectionHandle<S>, op: Op<S>) -> Result<ControlFlow<()>> {
        match op {
            Op::Send { encoded, version } => {
                // The bytes were encoded against a snapshot of the version. Refuse them if the
                // connection has moved on since, rather than writing an ID the peer resolves in
                // another table.
                if version != self.version {
                    return Err(ConnectionError::StaleEncoding {
                        packet: encoded.name,
                        encoded_version: version,
                        version: self.version,
                    });
                }
                trace!(packet = encoded.name, "writing packet");
                self.write(encoded).await?;
            }
            Op::Encrypt(cipher) => {
                debug!("enabling encryption");
                // The codec only encrypts what it encodes from here on, so bytes already buffered
                // stay plaintext, and no flush is needed to get the switchover point right.
                self.framed.codec_mut().set_cipher(cipher);
            }
            Op::SetVersion(version) => {
                debug!(%version, "updating version");
                self.version = version;
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
                    if Box::pin(self.handle_op(handle, op)).await?.is_break() {
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

    fn handle_tick(&mut self, handle: &ConnectionHandle<S>) -> Result<()> {
        self.dispatcher.on_tick(Ctx::new(&self.state, self.phase, self.version, &handle))
    }

    fn handle_frame(&mut self, handle: &ConnectionHandle<S>, frame: Frame) -> Result<()> {
        if self.exclusive > 0 {
            return Err(ConnectionError::EarlyPacket {
                phase: self.phase,
                id: frame.id,
            });
        }

        let ctx = Ctx::new(&self.state, self.phase, self.version, handle);
        self.dispatcher.on_frame(ctx, frame.id, &frame.payload)
    }

    /// Buffers a packet, under whichever deadline currently applies.
    ///
    /// The socket has to be written *somewhere*, and every candidate has the same problem: a peer
    /// that stops reading fills the write buffer and the write stops making progress. Doing it
    /// inside the loop's `select!` would mean re-entering a half-finished write on every wakeup,
    /// so it happens here instead -- with an escape, so a peer that will not read cannot outlast
    /// its deadline or ignore a shutdown.
    async fn write(&mut self, frame: Frame) -> Result<()> {
        let write = self.framed.feed(frame).map_err(Into::into);
        guarded(&self.shutdown, &mut self.lifetime, write).await
    }

    /// Writes whatever has been buffered, if anything, under the same guard as [`write`].
    async fn flush(&mut self) -> Result<()> {
        if self.framed.write_buffer().is_empty() {
            return Ok(());
        }
        let flush = self.framed.flush().map_err(Into::into);
        guarded(&self.shutdown, &mut self.lifetime, flush).await
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
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;

        result = future => Ok(result?),
        () = shutdown.cancelled() => Err(ConnectionError::Cancelled),
        () = expire(lifetime), if lifetime.is_some() => Err(ConnectionError::TimedOut),
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
        },
        None => std::future::pending().await,
    }
}
