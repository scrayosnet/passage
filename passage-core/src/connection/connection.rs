use crate::codec::{Frame, FrameCodec};
use crate::connection::{ConnectionError, ConnectionHandle, Ctx, Dispatcher, Op, Result};
use crate::phase::Phase;
use crate::version::ProtocolVersion;
use crate::wire::Options as WireOptions;
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

/// The default timeout for the `on_error` dispatcher hook.
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// The default maximum lifetime for a connection. The connection is closed gracefully is the peer
/// does not complete before this.
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(120);

/// The connection state at its completion. It contains the connection's final state and any error
/// that caused the completion.
#[derive(Debug)]
pub struct Outcome<S> {
    /// The connection completion cause if any.
    pub error: Option<ConnectionError>,

    /// The state is settled on.
    pub state: S,

    /// The protocol version it settled on.
    pub version: ProtocolVersion,

    /// The phase it settled on.
    pub phase: Phase,
}

/// The connection options.
#[derive(Copy, Clone, Debug)]
pub struct Options {
    /// The decoding options.
    pub wire_options: WireOptions,

    /// The interval between ticks. If unset, ticks are disabled.
    pub tick_interval: Option<Duration>,

    /// The maximum time the connection may live. If unset, the connection is not capped.
    pub max_lifetime: Option<Duration>,

    /// The maximum time the connection may gracefully shut down. If unset, the connection is not capped.
    pub close_timeout: Option<Duration>,

    /// The initial protocol version.
    pub initial_version: ProtocolVersion,

    /// The initial phase.
    pub initial_phase: Phase,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            wire_options: WireOptions::default(),
            tick_interval: None,
            max_lifetime: Some(DEFAULT_MAX_LIFETIME),
            close_timeout: Some(DEFAULT_CLOSE_TIMEOUT),
            initial_version: ProtocolVersion::UNKNOWN,
            initial_phase: Phase::Handshake,
        }
    }
}

/// Builds a new connection.
pub struct ConnectionBuilder<T, S, D> {
    io: T,
    dispatcher: D,
    state: S,
    config: Option<Options>,
    shutdown: Option<CancellationToken>,
}

impl<T, S, D> ConnectionBuilder<T, S, D> {
    /// Sets the configuration. Defaults to [`Options::default`].
    #[must_use]
    pub fn config(mut self, config: Options) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the connection's shutdown token. Defaults to [`CancellationToken::new`].
    #[must_use]
    pub fn shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = Some(shutdown);
        self
    }

    /// Sets the connection's dispatcher.
    #[must_use]
    pub fn dispatcher<D1>(self, dispatcher: D1) -> ConnectionBuilder<T, S, D1> {
        ConnectionBuilder {
            io: self.io,
            dispatcher,
            state: self.state,
            config: self.config,
            shutdown: self.shutdown,
        }
    }

    /// Sets the connection's state.
    #[must_use]
    pub fn state<S1>(self, state: S1) -> ConnectionBuilder<T, S1, D> {
        ConnectionBuilder {
            io: self.io,
            dispatcher: self.dispatcher,
            state,
            config: self.config,
            shutdown: self.shutdown,
        }
    }
}

impl<T, S, D> ConnectionBuilder<T, S, D>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
    D: Dispatcher<S>,
{
    /// Builds the connection from the builder.
    pub fn build(self) -> Connection<T, S, D> {
        Connection::new(
            self.io,
            self.dispatcher,
            self.state,
            self.config.unwrap_or_default(),
            self.shutdown.unwrap_or_default(),
        )
    }
}

/// Drives one connection.
pub struct Connection<T, S = (), D = ()> {
    /// The peer socket connection, wrapped by a framed codec.
    framed: Framed<T, FrameCodec>,

    /// The dispatcher that handles the incoming frames, ticks, and errors.
    dispatcher: D,

    /// The custom connection scoped state.
    state: S,

    /// The list of tasks that have been queued by the dispatcher.
    tasks: FuturesUnordered<BoxFuture<'static, (bool, Result<()>)>>,

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
    config: Options,
}

impl<T, S, D> Connection<T, S, D>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
    D: Dispatcher<S>,
{
    /// Creates a new connection builder.
    pub fn builder(io: T) -> ConnectionBuilder<T, (), ()> {
        ConnectionBuilder {
            io,
            dispatcher: (),
            state: (),
            config: None,
            shutdown: None,
        }
    }

    /// Creates a new connection.
    fn new(io: T, dispatcher: D, state: S, config: Options, shutdown: CancellationToken) -> Self {
        let ticker = config.tick_interval.map(|interval| {
            let mut ticker = tokio::time::interval_at(Instant::now() + interval, interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker
        });
        let lifetime = config
            .max_lifetime
            .map(|after| Box::pin(sleep_until(Instant::now() + after)));

        Self {
            framed: Framed::new(io, FrameCodec::new(config.wire_options)),
            dispatcher,
            state,
            tasks: FuturesUnordered::new(),
            exclusive: 0,
            version: config.initial_version,
            phase: config.initial_phase,
            ticker,
            lifetime,
            shutdown,
            config,
        }
    }

    /// Runs the connection to completion.
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

    /// Runs the connection loop. On error, it stops all handlers and starts the graceful shutdown.
    async fn serve(&mut self) -> Result<()> {
        // Run the handlers until they complete or raise an error.
        let (handle, mut ops) =
            ConnectionHandle::new(self.shutdown.clone(), self.config.wire_options);
        let Err(mut error) = self.drive(&handle, &mut ops).await else {
            return Ok(());
        };

        // Notify any detached tasks and re-create timeout limits for error hooks.
        self.tasks.clear();
        self.shutdown.cancel();

        // Restart the drive with the error handler.
        self.shutdown = CancellationToken::new();
        self.lifetime = self
            .config
            .close_timeout
            .map(|d| Box::pin(sleep_until(Instant::now() + d)));
        let (handle, mut ops) =
            ConnectionHandle::new(self.shutdown.clone(), self.config.wire_options);

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

    /// Drive the connection, using the provided handle and operations channel pair to schedule operations.
    async fn drive(
        &mut self,
        handle: &ConnectionHandle<S>,
        ops: &mut mpsc::UnboundedReceiver<Op<S>>,
    ) -> Result<()> {
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
                    return Err(ConnectionError::shutdown());
                },

                // 4. The deadline.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => {
                    return Err(ConnectionError::timeout());
                },

                // 5. Ticks -- but never while the peer must stay quiet. A keep-alive sent into a
                //    gated window would invite the very packet the gate rejects.
                () = tick(&mut self.ticker), if self.exclusive == 0 => {
                    self.handle_tick(handle)?;
                },

                // 6. Input. Polled even while gated: see the module docs.
                frame = self.framed.next() => {
                    let Some(frame) = frame else {
                        return Err(ConnectionError::peer());
                    };
                    self.handle_frame(handle, frame?)?;
                },
            }
        }
    }

    /// Handles a single operation. It returns [`ControlFlow::Break`] if the connection should be
    /// closed (without raising an error). On error, it returns without handing partially finished
    /// operations.
    async fn handle_op(
        &mut self,
        handle: &ConnectionHandle<S>,
        op: Op<S>,
    ) -> Result<ControlFlow<()>> {
        match op {
            Op::Send { encoded, version } => {
                // The bytes were encoded against a snapshot of the version. Refuse them if the
                // connection has moved on since, rather than writing an id the peer resolves in
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
                let ctx = Ctx::new(&self.state, self.phase, self.version, handle);
                self.dispatcher.on_version(ctx)?;
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

    /// Handles a tick.
    fn handle_tick(&mut self, handle: &ConnectionHandle<S>) -> Result<()> {
        Ok(self
            .dispatcher
            .on_tick(Ctx::new(&self.state, self.phase, self.version, &handle))?)
    }

    /// Handles a frame (i.e., incoming packet). It checks whether the exclusive guard is violated.
    fn handle_frame(&mut self, handle: &ConnectionHandle<S>, frame: Frame) -> Result<()> {
        if self.exclusive > 0 {
            return Err(ConnectionError::EarlyPacket {
                phase: self.phase,
                id: frame.id,
            });
        }

        let ctx = Ctx::new(&self.state, self.phase, self.version, handle);
        Ok(self.dispatcher.on_frame(ctx, frame.id, &frame.payload)?)
    }

    /// Writes a packet to the socket, guarded by the connection lifetime.
    async fn write(&mut self, frame: Frame) -> Result<()> {
        let write = self.framed.feed(frame).map_err(Into::into);
        guarded(&self.shutdown, &mut self.lifetime, write).await
    }

    /// Flushes the socket, guarded by the connection lifetime.
    async fn flush(&mut self) -> Result<()> {
        if self.framed.write_buffer().is_empty() {
            return Ok(());
        }
        let flush = self.framed.flush().map_err(Into::into);
        guarded(&self.shutdown, &mut self.lifetime, flush).await
    }
}

/// Runs a future guarded against a timeout lifetime and cancellation token.
async fn guarded<T>(
    shutdown: &CancellationToken,
    lifetime: &mut Option<Pin<Box<Sleep>>>,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;

        result = future => Ok(result?),
        () = shutdown.cancelled() => Err(ConnectionError::shutdown()),
        () = expire(lifetime), if lifetime.is_some() => Err(ConnectionError::timeout()),
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
