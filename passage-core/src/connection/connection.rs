use crate::codec::FrameCodec;
use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::{ConnCell, ConnRef, ConnectionError, DispatchError, Dispatcher, Out};
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
type Tasks<'a> = FuturesUnordered<BoxFuture<'a, Result<(), DispatchError>>>;

/// The default maximum lifetime for a connection. The connection is closed gracefully is the peer
/// does not complete before this.
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(120);

/// The connection state at its completion. It contains the connection's final state and any error
/// that caused the completion.
#[derive(Debug)]
pub struct Outcome<S> {
    /// The connection completion cause if any.
    pub error: Option<DispatchError>,

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

    /// The maximum time the connection may live. If unset, the connection is not capped. A handler
    /// may move the deadline; see [`Conn::set_deadline`](crate::connection::Conn::set_deadline).
    pub max_lifetime: Option<Duration>,

    /// The initial protocol version.
    pub initial_version: ProtocolVersion,

    /// The initial phase.
    pub initial_phase: Phase,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            wire_options: WireOptions::default(),
            max_lifetime: Some(DEFAULT_MAX_LIFETIME),
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

    /// The dispatcher that handles the connection opening, the incoming frames, and the ending.
    dispatcher: D,

    /// The custom connection scoped state. It is taken by [`run`](Connection::run) and shared with
    /// the dispatch tasks.
    state: Option<S>,

    /// The protocol version the codec and the dispatch table are currently bound to. It is used to
    /// detect a version change.
    version: ProtocolVersion,

    /// The deadline the timer below is armed for. It is used to detect that a handler moved it.
    deadline: Option<Instant>,

    /// The timer for the connection's lifetime.
    lifetime: Option<Pin<Box<Sleep>>>,

    /// The token the connection starts out ending on. The one it actually ends on lives in the
    /// cell, because a handler may replace it.
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
        let deadline = config.max_lifetime.map(|after| Instant::now() + after);

        Self {
            framed: Framed::new(io, FrameCodec::new(config.wire_options)),
            dispatcher,
            state: Some(state),
            version: config.initial_version,
            deadline,
            lifetime: deadline.map(|at| Box::pin(sleep_until(at))),
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

        // The limits live in the cell so that a handler can move them and answer them.
        cell.as_ref().with(|c| {
            c.set_shutdown(self.shutdown.clone());
            c.set_deadline(self.deadline);
        });

        // Run the connection loop, holding the task set every handler future lives in. The futures
        // borrow the cell, so dropping the set here leaves nobody able to write.
        let result = {
            let conn = cell.as_ref();
            let mut tasks = Tasks::new();
            let mut spare = Vec::new();
            self.drive(conn, &mut tasks, &mut spare).await
        };

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

    /// Drives the connection until it ends.
    async fn drive<'a>(
        &mut self,
        conn: ConnRef<'a, S>,
        tasks: &mut Tasks<'a>,
        spare: &mut Vec<Out>,
    ) -> Result<(), DispatchError> {
        // Bind the dispatcher to the version the connection starts at, then let it speak first.
        if let Err(error) = self.dispatcher.on_version(conn) {
            conn.with(|c| c.fail(error));
        }
        let opened = self.dispatcher.on_open(conn);
        if let Err(error) = start(opened, tasks).await {
            conn.with(|c| c.fail(error));
        }

        loop {
            // Follow whatever the handlers asked for: a version to rebind against, a deadline they
            // moved, a shutdown token they replaced.
            if let Err(error) = self.rebind(conn) {
                conn.with(|c| c.fail(error));
            }
            self.retime(conn);
            let shutdown = conn.with(|c| c.shutdown().clone());

            // Transmit all packets (and transmission options) to the peer, in the order they were
            // queued.
            if let Err(error) = self.transmit(conn, spare, &shutdown).await {
                conn.with(|c| c.fail(error));
            }

            // Everything queued has been written, so a connection that is ending now ends.
            if conn.with(|c| c.closing()) {
                return match conn.with(|c| c.take_error()) {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }

            tokio::select! {
                biased;

                // 1. Finished handler tasks, or something a running one queued.
                done = advance(tasks, conn), if !tasks.is_empty() => {
                    if let Some(Err(error)) = done {
                        conn.with(|c| c.fail(error));
                    }
                },

                // 2. Cancellation. A handler that wants to answer it awaits the token itself.
                () = shutdown.cancelled() => {
                    conn.with(|c| c.fail(ConnectionError::shutdown().into()));
                },

                // 3. The deadline, if configured. There is no time left to say anything.
                () = expire(&mut self.lifetime), if self.lifetime.is_some() => {
                    conn.with(|c| c.fail(ConnectionError::timeout().into()));
                },

                // 4. Next packet. The socket is polled even while the gate is shut, so a hangup is
                //    still noticed, but a frame that arrives while it is shut is a protocol break.
                frame = self.framed.next() => {
                    match frame.transpose() {
                        Ok(Some(frame)) => {
                            let gated = conn.with(|c| c.gated().then(|| c.phase()));
                            match gated {
                                Some(phase) => conn.with(|c| {
                                    let id = frame.id;
                                    c.fail(ConnectionError::EarlyPacket { phase, id }.into());
                                }),
                                None => {
                                    let handled =
                                        self.dispatcher.on_frame(conn, frame.id, frame.payload);
                                    if let Err(error) = start(handled, tasks).await {
                                        conn.with(|c| c.fail(error));
                                    }
                                },
                            }
                        },
                        Ok(None) => conn.with(|c| c.fail(ConnectionError::peer().into())),
                        Err(error) => conn.with(|c| c.fail(ConnectionError::from(error).into())),
                    }
                },
            }
        }
    }

    /// Writes everything the handlers queued, in the order they queued it.
    async fn transmit(
        &mut self,
        conn: ConnRef<'_, S>,
        spare: &mut Vec<Out>,
        shutdown: &CancellationToken,
    ) -> Result<(), DispatchError> {
        conn.with(|c| c.swap_out(spare));
        if spare.is_empty() {
            return Ok(());
        }

        for out in spare.drain(..) {
            match out {
                Out::Frame(frame) => {
                    trace!(packet = frame.name, "writing packet");
                    let write = self.framed.feed(frame).map_err(Into::into);
                    guarded(shutdown, &mut self.lifetime, write).await?;
                }
                Out::Cipher(cipher) => {
                    debug!("enabling encryption");
                    self.framed.codec_mut().set_cipher(cipher);
                }
            }
        }

        if self.framed.write_buffer().is_empty() {
            return Ok(());
        }
        let flush = self.framed.flush().map_err(Into::into);
        guarded(shutdown, &mut self.lifetime, flush).await
    }

    /// Tells the dispatcher to rebind if a handler moved the protocol version.
    fn rebind(&mut self, conn: ConnRef<'_, S>) -> Result<(), DispatchError> {
        let version = conn.with(|c| c.version());
        if version == self.version {
            return Ok(());
        }
        debug!(?version, "updating version");
        self.version = version;
        self.dispatcher.on_version(conn)
    }

    /// Re-arms the lifetime timer if a handler moved the deadline.
    fn retime(&mut self, conn: ConnRef<'_, S>) {
        let deadline = conn.with(|c| c.deadline());
        if deadline == self.deadline {
            return;
        }
        debug!(?deadline, "updating deadline");
        self.deadline = deadline;
        match deadline {
            // Resetting keeps the timer's allocation.
            Some(at) => match &mut self.lifetime {
                Some(sleep) => sleep.as_mut().reset(at),
                lifetime => *lifetime = Some(Box::pin(sleep_until(at))),
            },
            None => self.lifetime = None,
        }
    }
}

/// Polls the handler tasks, resolving when one finishes -- or, as `None`, when one of them queued
/// something for the peer and went back to waiting, which nothing else would wake the loop for.
async fn advance<'a, S>(
    tasks: &mut Tasks<'a>,
    conn: ConnRef<'_, S>,
) -> Option<Result<(), DispatchError>> {
    std::future::poll_fn(|cx| match tasks.poll_next_unpin(cx) {
        Poll::Ready(Some(done)) => Poll::Ready(Some(done)),
        // An empty set reports `Ready(None)` without registering a waker, so the arm this is polled
        // from is only enabled while the set has something in it.
        Poll::Ready(None) | Poll::Pending if conn.with(|c| c.queued()) => Poll::Ready(None),
        _ => Poll::Pending,
    })
    .await
}

/// Polls a future once, pushing it to the task queue if it is not ready. It is polled through
/// [`poll_fn`](std::future::poll_fn) so that a future which parks registers the real waker.
async fn start<'a>(
    mut future: BoxFuture<'a, Result<(), DispatchError>>,
    tasks: &mut Tasks<'a>,
) -> Result<(), DispatchError> {
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
    future: impl Future<Output = Result<T, DispatchError>>,
) -> Result<T, DispatchError> {
    tokio::select! {
        biased;

        result = future => Ok(result?),
        () = shutdown.cancelled() => Err(ConnectionError::shutdown().into()),
        () = expire(lifetime), if lifetime.is_some() => Err(ConnectionError::timeout().into()),
    }
}

/// Awaits a deadline, or never if there is none.
async fn expire(sleep: &mut Option<Pin<Box<Sleep>>>) {
    match sleep {
        Some(sleep) => sleep.as_mut().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[test]
    fn a_connection_nobody_configured_still_has_a_deadline() {
        // A peer that connects and then says nothing must not hold the socket forever.
        let options = Options::default();
        assert_eq!(options.max_lifetime, Some(DEFAULT_MAX_LIFETIME));
        assert_eq!(options.initial_version, ProtocolVersion::UNKNOWN);
        assert_eq!(options.initial_phase, Phase::Handshake);
    }

    #[tokio::test]
    async fn a_connection_starts_where_its_configuration_says() {
        // A client knows both before it says anything; a server learns them from the handshake.
        let options = Options {
            initial_phase: Phase::Status,
            initial_version: crate::versions::V26_1,
            ..Options::default()
        };
        let connection = Connection::<_, (), ()>::builder(duplex(64).0)
            .config(options)
            .build();

        // The phase lives in the cell, so it is read back from the outcome rather than a field.
        assert_eq!(connection.config.initial_phase, Phase::Status);
        assert_eq!(connection.version, crate::versions::V26_1);
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
        assert!(connection.deadline.is_none());
        assert!(connection.lifetime.is_none());
    }

    #[tokio::test]
    async fn awaiting_a_deadline_that_does_not_exist_waits_forever() {
        // An absent timer must never be ready, or the loop would spin on its arm.
        let mut nothing: Option<Pin<Box<Sleep>>> = None;
        assert!(
            tokio::time::timeout(Duration::from_millis(10), expire(&mut nothing))
                .await
                .is_err(),
        );
    }

    #[tokio::test]
    async fn a_guarded_future_loses_to_a_cancelled_token() {
        // Everything the connection writes goes through this, so a peer that stopped reading cannot
        // hold the loop past its deadline or past a shutdown.
        let pending = std::future::pending::<Result<(), DispatchError>>;

        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let error = guarded(&shutdown, &mut None, pending())
            .await
            .expect_err("the token is cancelled");
        assert_eq!(error.reason(), "shutdown");

        let mut expired: Option<Pin<Box<Sleep>>> = Some(Box::pin(sleep_until(
            Instant::now() - Duration::from_secs(1),
        )));
        let error = guarded(&CancellationToken::new(), &mut expired, pending())
            .await
            .expect_err("the deadline has passed");
        assert_eq!(error.reason(), "peer-timeout");
    }
}
