use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::{
    Connection, Dispatcher, MakeDispatcher, Options as ConnectionConfig, is_hangup,
};
use crate::router::{Layer, Stack};
use crate::server::listener::Listener;
use crate::wire::Options as WireOptions;
use futures::FutureExt;
use futures::future::BoxFuture;
use std::any::Any;
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{Instrument, Span, debug, error, field, info_span, trace, warn};

/// How long to wait before accepting again after an error that is not the peer's doing.
///
/// Running out of file descriptors is the case this exists for: it resolves itself as connections
/// close, so the loop must neither give up nor spin.
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// The [`Server`] accepts new connections using the configured listener, passes it through the
/// configured layers, and creates a new connection from it. The connection is handled in its separate
/// async context.
#[must_use = "a server does nothing until it is awaited"]
pub struct Server<L = (), F = (), M = (), A = ()> {
    /// The internal listener to accept new connections.
    listener: L,

    /// The state factory from which the state is build and passed to the connection.
    state: F,

    /// The dispatcher factory from which the dispatcher is build and passed to the connection.
    dispatcher: M,

    /// The layers to pass the connection through.
    layers: A,

    /// The configuration to pass to the connection.
    config: ConnectionConfig,

    /// The global shutdown signal. Each new connection gets a child token.
    shutdown: CancellationToken,

    /// How long to wait for connections to gracefully shut down before they are dropped.
    drain: Option<Duration>,

    /// The number of connections the server can accept at once.
    limit: Option<Arc<Semaphore>>,
}

impl<L: Listener> Server<L, (), (), ()> {
    /// Starts building a server that accepts from `listener`.
    ///
    /// The listener comes first and is not optional. Every other setter needs `L::Addr` in scope to
    /// be able to type its closure, so there is no useful server to build before it is known -- and
    /// a `Server` that has no listener is not a state worth being able to name.
    pub fn new(listener: L) -> Self {
        Server {
            listener,
            state: (),
            dispatcher: (),
            layers: (),
            config: ConnectionConfig::default(),
            shutdown: CancellationToken::new(),
            drain: None,
            limit: None,
        }
    }
}

impl<L: Listener, F, M, A: Layer<L::Io, L::Addr>> Server<L, F, M, A> {
    /// Sets the server listener.
    pub fn listener<L2>(self, listener: L2) -> Server<L2, F, M, A> {
        Server {
            listener,
            state: self.state,
            dispatcher: self.dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }

    /// Sets the server dispatch factory.
    pub fn dispatch<M2>(self, dispatcher: M2) -> Server<L, F, M2, A> {
        Server {
            listener: self.listener,
            state: self.state,
            dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }

    /// Sets the connection configuration.
    pub fn config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the wire options for the connection config.
    pub fn wire_options(mut self, wire_options: WireOptions) -> Self {
        self.config.wire_options = wire_options;
        self
    }

    /// Sets the max lifetime of the connection config.
    pub fn max_lifetime(mut self, after: Option<Duration>) -> Self {
        self.config.max_lifetime = after;
        self
    }

    /// Sets the initial protocol version of the connection config.
    pub fn initial_version(mut self, version: ProtocolVersion) -> Self {
        self.config.initial_version = version;
        self
    }

    /// Sets the initial phase of the connection config.
    pub fn initial_phase(mut self, phase: Phase) -> Self {
        self.config.initial_phase = phase;
        self
    }

    /// Sets the shutdown token for the server.
    pub fn graceful_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Sets the graceful shutdown timeout for the server.
    pub fn drain_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.drain = timeout;
        self
    }

    /// Sets the max connections for the server.
    pub fn max_connections(mut self, limit: Option<usize>) -> Self {
        self.limit = limit.map(|limit| Arc::new(Semaphore::new(limit)));
        self
    }

    /// Adds a new layer to the server, stacked behind the existing ones.
    pub fn layer<A2: Layer<A::Io, L::Addr>>(self, layer: A2) -> Server<L, F, M, Stack<A, A2>> {
        Server {
            listener: self.listener,
            state: self.state,
            dispatcher: self.dispatcher,
            layers: Stack::new(self.layers, layer),
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }

    /// Sets the state factory for the server.
    pub fn state<S, F2>(self, state: F2) -> Server<L, F2, M, A>
    where
        F2: Fn(&L::Addr) -> S,
    {
        Server {
            listener: self.listener,
            state,
            dispatcher: self.dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }
}

impl<L, S, F, M, A> Server<L, F, M, A>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Layer<L::Io, L::Addr>,
{
    /// Serves the [`Server`] to accept new connections. It completes once the shutdown token is
    /// canceled.
    ///
    /// Every connection holds a child of the shutdown token, so cancelling the server stops the
    /// accept loop *and* every live connection at once.
    /// [`drain_timeout`](Server::drain_timeout) bounds how long the server waits for all of them.
    pub async fn serve(mut self) {
        // The shared server state, used to create new connections.
        let tasks = TaskTracker::new();
        let shared = Arc::new(Shared {
            state: self.state,
            layers: self.layers,
            config: self.config,
        });

        loop {
            // Ensure that the server is allowed to accept new connections. Otherwise, wait until it
            // is able to do so.
            let permit = match &self.limit {
                None => None,
                Some(limit) => Some(tokio::select! {
                    biased;
                    () = self.shutdown.cancelled() => break,
                    permit = Arc::clone(limit).acquire_owned() =>
                        permit.expect("the semaphore is never closed"),
                }),
            };

            // Accept a new peer connection. If accepting a connection fails (unrelated to the peer),
            // then the server waits for its configured backoff.
            let (io, addr) = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => break,
                accepted = self.listener.accept() => match accepted {
                    Ok(accepted) => {
                        // Trace, not debug: this fires for every scanner on the internet, and the
                        // connection span below is what an operator actually follows.
                        trace!(address = ?accepted.1, "accepted a connection");
                        accepted
                    },
                    Err(err) if is_peer_error(&err) => {
                        debug!(cause = %err, "the peer went away before we accepted it");
                        continue;
                    }
                    Err(err) => {
                        warn!(cause = %err, "failed to accept; pausing");
                        tokio::select! {
                            biased;
                            () = self.shutdown.cancelled() => break,
                            () = tokio::time::sleep(ACCEPT_BACKOFF) => continue,
                        }
                    }
                },
            };

            // Build the new connection state and move it into its own thread. This also includes
            // layers such as the rate limiter as they might wait for client messages which should
            // not block the acceptance loop.
            let started = Instant::now();
            let dispatcher = self.dispatcher.make();
            let shutdown = self.shutdown.child_token();
            let span = info_span!(
                "connection",
                otel.kind = "server",
                otel.status_code = field::Empty,
                network.protocol.name = "minecraft",
                network.protocol.version = field::Empty,
                error.type = field::Empty,
                peer = field::Empty,
            );
            tasks.spawn(
                connection::<L, S, F, M::Dispatcher, A>(
                    Arc::clone(&shared),
                    io,
                    addr,
                    dispatcher,
                    shutdown,
                    started,
                    permit,
                )
                .instrument(span),
            );
        }

        // Close the task pool and start draining.
        tasks.close();
        debug!(connections = tasks.len(), "draining");
        match self.drain {
            None => tasks.wait().await,
            Some(after) => {
                if tokio::time::timeout(after, tasks.wait()).await.is_err() {
                    warn!(connections = tasks.len(), "the drain timeout expired");
                }
            }
        }
    }
}

impl<L, S, F, M, A> IntoFuture for Server<L, F, M, A>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Layer<L::Io, L::Addr>,
{
    type Output = ();
    // Boxed because there is nothing else to write here: the future is an `async fn` body, whose
    // type has no name, and naming it would need `impl Future` in an associated type position --
    // which is still unstable. `run` is the same future, unboxed, for anyone who minds.
    type IntoFuture = BoxFuture<'static, ()>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.serve())
    }
}

/// What every connection task needs from the server, shared by all of them.
struct Shared<F, A> {
    state: F,
    layers: A,
    config: ConnectionConfig,
}

/// Handles a single accepted connection.
async fn connection<L, S, F, D, A>(
    shared: Arc<Shared<F, A>>,
    io: L::Io,
    addr: L::Addr,
    dispatcher: D,
    shutdown: CancellationToken,
    started: Instant,
    permit: Option<OwnedSemaphorePermit>,
) where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    D: Dispatcher<S> + Send + 'static,
    A: Layer<L::Io, L::Addr>,
{
    // Apply the layer stack. If the layer stack is empty, then the connection is just a raw I/O.
    // Afterward, the final peer address is recorded.
    let _permit = permit;
    let span = Span::current();
    let Some((io, addr)) = shared.layers.admit(io, addr).await else {
        return;
    };
    span.record("peer", field::debug(&addr));

    // Build and run the connection and log the result.
    let connection = Connection::<_, (), ()>::builder(io)
        .dispatcher(dispatcher)
        .state((shared.state)(&addr))
        .config(shared.config)
        .shutdown(shutdown)
        .build();
    match AssertUnwindSafe(connection.run()).catch_unwind().await {
        Ok(outcome) => {
            let elapsed = started.elapsed();
            let version = outcome.version;
            let phase = outcome.phase;
            span.record("network.protocol.version", field::display(version));
            match &outcome.error {
                // A handler closed it. There is nothing to report.
                None => debug!(?elapsed, ?version, ?phase, "connection closed"),
                // A hangup, a timeout, a peer that sent nonsense. All ordinary weather, and a
                // scanner that took its MOTD and left looks exactly like the first of them.
                Some(err) if err.is_peer_error() => {
                    span.record("error.type", err.reason());
                    debug!(
                        ?elapsed,
                        ?version,
                        ?phase,
                        reason = err.reason(),
                        "connection ended"
                    );
                }
                // Our bug, or a dependency failing: the one case an operator has to see.
                Some(err) => {
                    span.record("otel.status_code", "error");
                    span.record("error.type", err.reason());
                    error!(
                        ?elapsed,
                        ?version,
                        ?phase,
                        reason = err.reason(),
                        cause = %err,
                        "connection failed"
                    );
                }
            }
        }
        Err(payload) => {
            // `as_ref`, not `&payload`: a `&Box<dyn Any>` coerces to a `dyn Any` whose concrete
            // type is the *box*, so every downcast inside would miss and every panic would be
            // reported as the fallback text.
            span.record("otel.status_code", "error");
            span.record("error.type", "panic");
            error!(
                cause = panic_message(payload.as_ref()),
                "connection panicked"
            );
        }
    };
}

/// Tries tries generate an error message from a panic.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a handler panicked".to_owned()
    }
}

/// Checks whether the io error is raised by the peer.
///
/// The same endings a live connection can meet, plus the one only accepting has: a signal that
/// interrupted the call, which costs us nothing but the connection that was half-way in.
fn is_peer_error(err: &io::Error) -> bool {
    is_hangup(err) || err.kind() == io::ErrorKind::Interrupted
}
