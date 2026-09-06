//! The accept loop: one [`Connection`] per accepted socket.
//!
//! This is the piece that turns a [`Router`] -- static, built once, shared -- into a running
//! server. It is deliberately thin: accept, build the state for that peer, hand both to a
//! [`Connection`], and keep track of the task so shutdown can wait for it.
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, log_completion, router};
//! use passage_driver::server::serve;
//! use std::sync::Arc;
//! use tokio::net::TcpListener;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("0.0.0.0:25565").await?;
//!
//! serve(listener, Arc::new(router()?), |addr| Session {
//!     peer: Some(*addr),
//!     ..Session::default()
//! })
//! .on_finish(log_completion)
//! .await;
//! # Ok(())
//! # }
//! ```
//!
//! # State is per connection, not shared
//!
//! This is where it differs from axum, whose `State` is one value shared by every request. Here
//! `S` is the connection's own state -- the hostname it asked for, the profile it authenticated
//! as, the keep-alive it owes an answer to -- so `serve` takes a *factory* rather than a value, and
//! calls it once per accepted socket with the peer's address. Anything genuinely shared belongs in
//! a captured [`Arc`] instead, either in that closure or in the handlers themselves.
//!
//! # What runs where
//!
//! Only the accept itself happens in the loop. Everything that touches a particular socket --
//! [`Listener::prepare`], the admission check, the state factory, the protocol -- runs on that
//! connection's own task, because all of them can wait on the peer and none of them may hold up
//! the next accept.
//!
//! # Writing your own loop
//!
//! Nothing here is privileged: it is a hundred lines over the public [`Connection`] API. Reach for
//! your own loop when you need something this does not offer -- a listener that is not one socket,
//! admission control that has to see the first packet, several routers behind one port.

use crate::conn::{Connection, ConnectionConfig, Dispatcher, Ending, Outcome};
use crate::error::Error;
use crate::router::{Router, RouterDispatcher};
use futures::future::BoxFuture;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, trace, warn};

/// How long to wait before accepting again after an error that is not the peer's doing.
///
/// Running out of file descriptors is the case this exists for: it resolves itself as connections
/// close, so the loop must neither give up nor spin.
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// A source of connections.
///
/// Implemented for [`TcpListener`]. Implement it yourself to serve something else -- a Unix socket,
/// a socket pair in a test, or a transport that needs a handshake of its own.
pub trait Listener: Send + 'static {
    /// The socket this listener produces.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// How a peer is identified. Handed to the state factory for every accepted connection.
    type Addr: fmt::Debug + Send + 'static;

    /// Accepts the next connection.
    ///
    /// Returning an error must be safe to retry: the loop logs it and calls `accept` again.
    fn accept(&mut self) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send;

    /// Prepares an accepted socket, **on that connection's own task**.
    ///
    /// This is where a preamble belongs: a PROXY protocol header, a TLS handshake, anything that
    /// has to read from the socket before the protocol starts. Doing it in
    /// [`accept`](Listener::accept) would be a mistake -- the loop awaits that one call, so a peer
    /// that connects and then says nothing would hold up every other connection to the server.
    ///
    /// Returning a different address is the point of the PROXY case: what comes back is what the
    /// state factory and the admission check see.
    ///
    /// The default does nothing.
    fn prepare(
        io: Self::Io,
        addr: Self::Addr,
    ) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send {
        std::future::ready(Ok((io, addr)))
    }
}

impl Listener for TcpListener {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        TcpListener::accept(self).await
    }
}

/// Makes the [`Dispatcher`] for one accepted connection.
///
/// Implemented for `Arc<Router<S>>`, which is the ordinary case: every connection gets a
/// [`RouterDispatcher`] over the same shared router. Implement it -- or use [`make_with`] -- to
/// serve something that is not a router.
pub trait MakeDispatcher<S>: Send + 'static {
    /// The dispatcher this produces.
    type Dispatcher: Dispatcher<S> + Send + 'static;

    /// Makes one, for one connection.
    fn make(&self) -> Self::Dispatcher;
}

impl<S: 'static> MakeDispatcher<S> for Arc<Router<S>> {
    type Dispatcher = RouterDispatcher<S>;

    fn make(&self) -> RouterDispatcher<S> {
        RouterDispatcher::new(Arc::clone(self))
    }
}

/// A [`MakeDispatcher`] built from a closure. See [`make_with`].
pub struct MakeWith<F>(F);

impl<S, D, F> MakeDispatcher<S> for MakeWith<F>
where
    F: Fn() -> D + Send + 'static,
    D: Dispatcher<S> + Send + 'static,
{
    type Dispatcher = D;

    fn make(&self) -> D {
        (self.0)()
    }
}

/// Makes a dispatcher per connection from a closure.
///
/// ```ignore
/// serve(listener, make_with(move || Recorder::new(Arc::clone(&log))), |_| ());
/// ```
pub fn make_with<F>(make: F) -> MakeWith<F> {
    MakeWith(make)
}

/// What a finished connection reports to [`Server::on_finish`].
///
/// The connection's own [`Outcome`] carries the state, the version and the phase it reached; what
/// only the server knows is added here.
pub struct Finished<'a, S, A> {
    /// How long the connection lasted, from the accept to the last byte.
    pub elapsed: Duration,

    /// The peer, as reported by [`Listener::prepare`]. Always known: it belongs to the task that
    /// reports, not to the one that ran the protocol, so even a panic cannot take it with it.
    pub addr: &'a A,

    /// How it ended, in the connection's own terms. A task that panicked is folded in here as an
    /// [`Ending::Failed`] with an internal error, so there is one thing to match on rather than a
    /// special case that only the server knows about.
    pub result: &'a std::result::Result<(), Ending>,

    /// What the connection knew about itself -- its state, version and phase -- or [`None`] if its
    /// task panicked before it could say.
    pub outcome: Option<&'a Outcome<S>>,
}

impl<S, A> Finished<'_, S, A> {
    /// The state the connection was left in, unless it panicked.
    #[must_use]
    pub fn state(&self) -> Option<&S> {
        self.outcome.map(|outcome| &outcome.state)
    }
}

/// What runs when a connection ends. Shared, because every connection reports to the same one.
type OnFinish<S, A> = Arc<dyn Fn(&Finished<'_, S, A>) + Send + Sync>;

/// Whether a peer is allowed in at all.
type OnAccept<A> = Arc<dyn Fn(&A) -> bool + Send + Sync>;

/// Accepts connections and runs one [`Connection`] per socket.
///
/// Created by [`serve`]. Configure it with the builder methods, then `await` it -- or call
/// [`run`](Server::run), which is the same thing without boxing the future.
pub struct Server<L: Listener, S, F, M> {
    listener: L,
    make: M,
    state: Arc<F>,
    config: ConnectionConfig,
    shutdown: CancellationToken,
    drain: Option<Duration>,
    limit: Option<Arc<Semaphore>>,
    on_accept: Option<OnAccept<L::Addr>>,
    on_finish: Option<OnFinish<S, L::Addr>>,
}

/// Serves `make`'s dispatchers on `listener`, building per-connection state with `state`.
///
/// `make` is usually an `Arc<Router<S>>`; `state` is called once per accepted socket, with the
/// address the listener reported -- `|_| S::default()` when the peer is of no interest.
///
/// Nothing happens until the returned [`Server`] is awaited.
#[must_use = "a server does nothing until it is awaited"]
pub fn serve<L, S, F, M>(listener: L, make: M, state: F) -> Server<L, S, F, M>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
{
    Server {
        listener,
        make,
        state: Arc::new(state),
        config: ConnectionConfig::default(),
        shutdown: CancellationToken::new(),
        drain: None,
        limit: None,
        on_accept: None,
        on_finish: None,
    }
}

impl<L, S, F, M> Server<L, S, F, M>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
{
    /// Sets the configuration every connection is created with.
    #[must_use]
    pub fn config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Stops accepting when `shutdown` is cancelled, then waits for live connections to finish.
    ///
    /// Waiting is the point: a connection that is mid-transfer gets to send its transfer packet.
    /// What bounds that wait is [`ConnectionConfig::max_lifetime`], or a
    /// [`drain_timeout`](Server::drain_timeout) if you would rather cap it here.
    #[must_use]
    pub fn with_graceful_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Cancels connections that are still live this long after the shutdown signal.
    ///
    /// Without it, `serve` waits for every connection to end on its own.
    #[must_use]
    pub fn drain_timeout(mut self, after: Duration) -> Self {
        self.drain = Some(after);
        self
    }

    /// Caps how many connections may be live at once.
    ///
    /// The slot is taken *before* the accept, so a server at its limit leaves the next connection
    /// in the kernel's backlog instead of accepting it only to drop it -- which is the difference
    /// between backpressure and a refusal the peer cannot distinguish from an outage.
    #[must_use]
    pub fn max_connections(mut self, limit: usize) -> Self {
        self.limit = Some(Arc::new(Semaphore::new(limit)));
        self
    }

    /// Decides whether a peer is served at all, once its real address is known.
    ///
    /// This is where per-IP rate limiting goes. It runs on the connection's own task, after
    /// [`Listener::prepare`], so it sees the address a PROXY header reported rather than the
    /// immediate peer's -- and a slow preamble cannot hold up the accept loop. Returning `false`
    /// closes the socket without a byte of protocol.
    #[must_use]
    pub fn on_accept(
        mut self,
        on_accept: impl Fn(&L::Addr) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.on_accept = Some(Arc::new(on_accept));
        self
    }

    /// Runs `on_finish` for every connection that ends, with what it ended on.
    ///
    /// This is the reporting hook: what to log, and how loudly, is the caller's policy, so `serve`
    /// itself logs nothing about how a connection went. A connection whose task *panicked* is
    /// reported here too, as an internal error -- otherwise it would be the one outcome that never
    /// reached the metrics. See `demo::server::log_completion` for the shape it expects.
    #[must_use]
    pub fn on_finish(
        mut self,
        on_finish: impl Fn(&Finished<'_, S, L::Addr>) + Send + Sync + 'static,
    ) -> Self {
        self.on_finish = Some(Arc::new(on_finish));
        self
    }

    /// Accepts until the shutdown signal, then drains.
    ///
    /// An accept error is never fatal. One that is the peer's doing (it hung up between the SYN and
    /// our `accept`) is logged at `trace` and retried at once; anything else is logged at `warn`
    /// and retried after a pause, because the failure that matters in practice -- no file
    /// descriptors left -- is transient, and a server that exits on it turns a busy minute into an
    /// outage. A listener that stays broken keeps saying so once a second, which is a supervision
    /// signal rather than something a protocol server can fix by giving up.
    pub async fn run(mut self) {
        // Independent of the shutdown token rather than a child of it, which is what makes the
        // drain below graceful: cancelling the server stops accepts, not connections. The guard is
        // what makes dropping this future safe -- without it, a `select!` that loses to a signal
        // would leave every live connection running with nothing able to stop it.
        let live = CancelOnDrop(CancellationToken::new());
        let tasks = TaskTracker::new();

        loop {
            // Taken before the accept, so a full server stops accepting rather than accepting to
            // refuse. Held by the connection task until it ends.
            let permit = match &self.limit {
                None => None,
                Some(limit) => Some(tokio::select! {
                    biased;
                    () = self.shutdown.cancelled() => break,
                    permit = Arc::clone(limit).acquire_owned() =>
                        permit.expect("the semaphore is never closed"),
                }),
            };

            let accepted = tokio::select! {
                biased;

                () = self.shutdown.cancelled() => break,

                accepted = self.listener.accept() => accepted,
            };

            let (io, addr) = match accepted {
                Ok(accepted) => accepted,
                Err(err) if is_peer_error(&err) => {
                    trace!(cause = %err, "the peer went away before we accepted it");
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
            };

            trace!(?addr, "accepted a connection");
            // One dispatcher per connection: for a router, two `Arc` clones, and the table it
            // caches then follows the version this connection negotiates.
            let dispatcher = self.make.make();
            let state = Arc::clone(&self.state);
            let config = self.config.clone();
            // Its own token, so a connection ending cancels itself and nothing else.
            let shutdown = live.0.child_token();
            let on_accept = self.on_accept.clone();
            let on_finish = self.on_finish.clone();

            tasks.spawn(async move {
                let started = Instant::now();
                let _permit = permit;

                let (io, addr) = match L::prepare(io, addr).await {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        debug!(cause = %err, "failed to prepare an accepted socket");
                        return;
                    }
                };

                if let Some(on_accept) = on_accept
                    && !on_accept(&addr)
                {
                    debug!(?addr, "refused a connection before the protocol started");
                    return;
                }

                let (connection, _handle) =
                    Connection::new(io, dispatcher, state(&addr), config, shutdown);

                // On its own task so that a panic in a handler is something we can *see*: a panic
                // on this task would unwind straight past the report below, and the connection
                // would simply vanish from whatever `on_finish` feeds.
                let outcome = tokio::spawn(connection.run()).await;

                let Some(on_finish) = on_finish else {
                    return;
                };
                let panicked;
                let (outcome, result) = match &outcome {
                    Ok(outcome) => (Some(outcome), &outcome.result),
                    Err(err) => {
                        panicked = Err(Ending::Failed(Error::internal("panic", err.to_string())));
                        (None, &panicked)
                    }
                };
                on_finish(&Finished {
                    elapsed: started.elapsed(),
                    addr: &addr,
                    result,
                    outcome,
                });
            });
        }

        // Nothing new will be accepted, so the tracker can only shrink from here.
        tasks.close();
        debug!(connections = tasks.len(), "draining");

        match self.drain {
            None => tasks.wait().await,
            Some(after) => {
                if tokio::time::timeout(after, tasks.wait()).await.is_err() {
                    debug!(
                        connections = tasks.len(),
                        "the drain timeout expired; cancelling what is left"
                    );
                    live.0.cancel();
                    tasks.wait().await;
                }
            }
        }
    }
}

impl<L, S, F, M> IntoFuture for Server<L, S, F, M>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
{
    type Output = ();
    type IntoFuture = BoxFuture<'static, ()>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

/// A cancellation token that fires if it is dropped without being awaited.
///
/// The connections are children of it, so letting it go quietly would orphan them.
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Whether an accept error says something about the peer rather than about us.
///
/// A connection that is reset or aborted between the SYN and our `accept` is ordinary internet
/// weather, and nothing about it suggests waiting will help.
fn is_peer_error(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::Interrupted
    )
}
