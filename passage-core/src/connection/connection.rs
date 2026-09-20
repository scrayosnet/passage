use crate::codec::{Frame, FrameCodec};
use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::{
    Conn, ConnCell, ConnRef, ConnectionError, DispatchError, Dispatcher, Out, Result,
};
use crate::wire::Options as WireOptions;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{SinkExt, StreamExt, TryFutureExt};

use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Instant, Sleep, sleep_until};
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace};

/// The (uncompleted) dispatch tasks for a connection. They borrow a [`ConnCell`] to interact with the
/// peer.
type Tasks<'a> = FuturesUnordered<BoxFuture<'a, std::result::Result<(), DispatchError>>>;

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

    /// The custom connection scoped state. It is taken by [`run`](Connection::run) and shared with
    /// the dispatch tasks.
    state: Option<S>,

    /// The protocol version the codec and the dispatch table are currently bound to. It is used to
    /// detect a version change.
    version: ProtocolVersion,

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
            state: Some(state),
            version: config.initial_version,
            ticker,
            lifetime,
            shutdown,
            config,
        }
    }

    /// Runs the connection to completion.
    pub async fn run(mut self) -> Outcome<S> {
        // Creates a run-scoped connection cell with the connection state. The state is only ever
        // taken here. This cell will be used by the dispatch handlers.
        let cell = ConnCell::new(
            self.state.take().expect("a connection is run once"),
            self.config.initial_version,
            self.config.initial_phase,
            self.config.wire_options,
        );

        let result = self.serve(&cell).await;

        // Close the socket under a token of its own. This requires a new cancellation token as the
        // connection cancellation token might already be canceled at this point (i.e., shutdown).
        let closing = CancellationToken::new();
        let close = self.framed.close().map_err(Into::into);
        if let Err(error) = guarded(&closing, &mut self.lifetime, close).await {
            debug!(%error, "failed to close the socket");
        }

        // Unwrap the connection data from the run-scoped connection. At this point, all dispatch
        // handlers have completed (or dropped).
        let conn = cell.into_inner();
        Outcome {
            error: result.err(),
            version: conn.version(),
            phase: conn.phase(),
            state: conn.state,
        }
    }

    /// Runs the connection loop. On error, it stops all handlers and starts the graceful shutdown.
    async fn serve(&mut self, cell: &ConnCell<S>) -> Result<()> {
        let conn = cell.as_ref();
        let mut tasks = Tasks::new();
        let mut spare = Vec::new();

        // Run the handlers until they complete or raise an error. `on_open` runs before the loop so
        // that a peer which speaks first (e.g., a client) is able to.
        let result = match self.dispatcher.on_open(conn) {
            Ok(()) => self.drive(conn, &mut tasks, &mut spare).await,
            Err(error) => Err(error.into()),
        };
        let Err(mut error) = result else {
            return Ok(());
        };

        // Drop every (dispatch) handler. They will never be called again, ensuring that noone except
        // the error hook writes to the connection. Everything already scheduled for the peer is kept
        // in the queue to ensure consistency between what the peer gets and the connection state.
        tasks.clear();

        // The error hook gets its own timeout.
        self.shutdown = CancellationToken::new();
        self.lifetime = self
            .config
            .close_timeout
            .map(|d| Box::pin(sleep_until(Instant::now() + d)));
        if let Err(error) = self.dispatcher.on_error(conn, &mut error) {
            debug!(%error, "failed to complete error handling");
        }
        self.settle(conn, &mut spare).await;

        Err(error)
    }

    /// Carries out what the error hook queued without accepting new frames or handling ticks.
    ///
    /// Failures here are logged and stop the drain. The reason the connection is ending was decided
    /// before this ran, and losing it to a broken socket on the way out would replace the diagnosis
    /// with the symptom.
    async fn settle(&mut self, conn: ConnRef<'_, S>, spare: &mut Vec<Out>) {
        if let Err(error) = self.transmit(conn, spare).await {
            debug!(%error, "gave up on what the error handler queued");
        }
    }

    /// Drives the connection until it ends.
    async fn drive<'a>(
        &mut self,
        conn: ConnRef<'a, S>,
        tasks: &mut Tasks<'a>,
        spare: &mut Vec<Out>,
    ) -> Result<()> {
        loop {
            // Update the connection and dispatcher version to match the version set by the dispatch
            // handlers.
            self.rebind(conn)?;

            // Transmit all packets (and transmission options) to the peer. After flushing, close the
            // connection if requested.
            self.transmit(conn, spare).await?;
            if conn.with(|c| c.closing()) {
                return Ok(());
            }

            tokio::select! {
                biased;

                // 1. Finished handler tasks.
                done = tasks.next(), if !tasks.is_empty() => {
                    done.expect("the set is not empty")?;
                },

                // 2. Cancellation.
                () = self.shutdown.cancelled() => {
                    return Err(ConnectionError::shutdown());
                },

                // 3. The deadline, if configured.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => {
                    return Err(ConnectionError::timeout());
                },

                // 4. Ticks, if allowed.
                () = tick(&mut self.ticker), if !conn.with(|c| c.gated()) => {
                    let ticked = self.dispatcher.on_tick(conn);
                    start(ticked, tasks).await?;
                },

                // 5. Next packet, if allowed.
                frame = self.framed.next(), if !conn.with(|c| c.gated()) => {
                    let Some(frame) = frame.transpose()? else {
                        return Err(ConnectionError::peer());
                    };
                    let handled = self.dispatcher.on_frame(conn, frame.id, &frame.payload);
                    start(handled, tasks).await?
                },
            }
        }
    }

    /// Writes everything the handlers queued, in the order they queued it.
    async fn transmit(&mut self, conn: ConnRef<'_, S>, spare: &mut Vec<Out>) -> Result<()> {
        conn.with(|c| c.swap_out(spare));
        if spare.is_empty() {
            return Ok(());
        }

        for out in spare.drain(..) {
            match out {
                Out::Frame(frame) => {
                    trace!(packet = frame.name, "writing packet");
                    self.write(frame).await?;
                }
                Out::Cipher(cipher) => {
                    debug!("enabling encryption");
                    self.framed.codec_mut().set_cipher(cipher);
                }
            }
        }
        self.flush().await
    }

    /// Tells the dispatcher to rebind if a handler moved the protocol version.
    fn rebind(&mut self, conn: ConnRef<'_, S>) -> Result<()> {
        let version = conn.with(|c| c.version());
        if version == self.version {
            return Ok(());
        }
        debug!(%version, "updating version");
        self.version = version;
        Ok(self.dispatcher.on_version(conn)?)
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

/// Polls a future once, pushing it to the task queue if it is not ready.
///
/// It is polled through [`poll_fn`](std::future::poll_fn) rather than a no-op waker, because a
/// future that parks has to register the *real* waker or nothing would ever wake it again.
async fn start<'a>(
    mut future: BoxFuture<'a, std::result::Result<(), DispatchError>>,
    tasks: &mut Tasks<'a>,
) -> Result<()> {
    let polled = std::future::poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx))).await;
    match polled {
        Poll::Ready(result) => Ok(result?),
        Poll::Pending => {
            tasks.push(future);
            Ok(())
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[test]
    fn a_connection_nobody_configured_still_has_a_deadline() {
        // The documented minimal setup is `Options::default()`, so that is where the security
        // posture lives: a peer that connects and then says nothing must not hold the socket
        // forever, and the last word must be bounded too.
        let options = Options::default();
        assert_eq!(options.max_lifetime, Some(DEFAULT_MAX_LIFETIME));
        assert_eq!(options.close_timeout, Some(DEFAULT_CLOSE_TIMEOUT));
        assert_eq!(
            options.tick_interval, None,
            "ticks are the caller's to ask for"
        );
        assert_eq!(options.initial_version, ProtocolVersion::UNKNOWN);
        assert_eq!(options.initial_phase, Phase::Handshake);
    }

    #[tokio::test]
    async fn a_connection_starts_where_its_configuration_says() {
        // A client knows both before it says anything; a server learns them from the handshake.
        let options = Options {
            initial_phase: Phase::Status,
            initial_version: crate::versions::V26_1,
            tick_interval: Some(Duration::from_secs(16)),
            ..Options::default()
        };
        let connection = Connection::<_, (), ()>::builder(duplex(64).0)
            .config(options)
            .build();

        // The phase is not the connection's to hold any more -- it lives in the cell handlers are
        // lent, so it is read back from the outcome rather than from a field.
        assert_eq!(connection.config.initial_phase, Phase::Status);
        assert_eq!(connection.version, crate::versions::V26_1);
        assert!(connection.ticker.is_some());
        assert!(connection.lifetime.is_some());

        let outcome = connection.run().await;
        assert_eq!(outcome.phase, Phase::Status, "and nothing moved it");
        assert_eq!(outcome.version, crate::versions::V26_1);
    }

    #[tokio::test]
    async fn a_timer_nobody_asked_for_is_not_armed() {
        let connection = Connection::<_, (), ()>::builder(duplex(64).0)
            .config(Options {
                max_lifetime: None,
                ..Options::default()
            })
            .build();
        assert!(connection.ticker.is_none());
        assert!(connection.lifetime.is_none());
    }

    #[tokio::test]
    async fn awaiting_a_deadline_that_does_not_exist_waits_forever() {
        // What the `if` guards in the loop rely on: an absent timer must never be ready, or the
        // connection would spin on the arm that has nothing to report.
        let mut nothing: Option<Pin<Box<Sleep>>> = None;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), expire(&mut nothing))
                .await
                .is_err(),
        );
        let mut never: Option<tokio::time::Interval> = None;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), tick(&mut never))
                .await
                .is_err(),
        );
    }

    #[tokio::test]
    async fn a_guarded_future_loses_to_a_cancelled_token() {
        // Everything the connection writes goes through this, so a peer that stopped reading cannot
        // hold the loop past its deadline or past a shutdown.
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let error = guarded(&shutdown, &mut None, std::future::pending::<Result<()>>())
            .await
            .expect_err("the token is cancelled");
        assert!(matches!(error, ConnectionError::Closed { .. }), "{error}");

        let mut expired: Option<Pin<Box<Sleep>>> = Some(Box::pin(sleep_until(
            Instant::now() - Duration::from_secs(1),
        )));
        let error = guarded(
            &CancellationToken::new(),
            &mut expired,
            std::future::pending::<Result<()>>(),
        )
        .await
        .expect_err("the deadline has passed");
        assert_eq!(error.reason(), "peer-timeout");
    }
}
