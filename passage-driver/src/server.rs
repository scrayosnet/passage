//! The accept loop: one [`Connection`] per accepted socket.
//!
//! This is the piece that turns a [`Router`] -- static, built once, shared -- into a running
//! server. It is deliberately thin: accept, build the state for that peer, hand both to a
//! [`Connection`], and keep track of the task so shutdown can wait for it.
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, log_completion, router};
//! use passage_driver::server::serve;
//! use tokio::net::TcpListener;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("0.0.0.0:25565").await?;
//!
//! serve(listener, router()?, |addr| Session {
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
//! # Writing your own loop
//!
//! Nothing here is privileged: it is 60 lines over the public [`Connection`] API. Reach for your
//! own loop when you need something this does not offer -- an accept semaphore, per-peer rate
//! limiting, a listener that is not one socket. [`Listener`] is the smaller extension point: a
//! wrapper that parses a PROXY protocol header and reports the real client address is an
//! implementation of it, and everything above keeps working.

use crate::conn::{Completion, Connection, ConnectionConfig};
use crate::error::Result;
use crate::router::{Router, RouterDispatcher};
use futures::future::BoxFuture;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
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
/// a socket pair in a test, or a wrapper that unwraps a PROXY protocol header and reports the
/// address it found instead of the immediate peer's.
pub trait Listener: Send + 'static {
    /// The socket this listener produces.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// How a peer is identified. Handed to the state factory for every accepted connection.
    type Addr: fmt::Debug + Send + 'static;

    /// Accepts the next connection.
    ///
    /// Returning an error must be safe to retry: the loop logs it and calls `accept` again.
    fn accept(&mut self) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send;
}

impl Listener for TcpListener {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        TcpListener::accept(self).await
    }
}

/// What runs when a connection ends. Shared, because every connection reports to the same one.
type OnFinish = Arc<dyn Fn(&Result<Completion>) + Send + Sync>;

/// Accepts connections and runs one [`Connection`] per socket.
///
/// Created by [`serve`]. Configure it with the builder methods, then `await` it -- or call
/// [`run`](Server::run), which is the same thing without boxing the future.
pub struct Server<L: Listener, S, F> {
    listener: L,
    router: Arc<Router<S>>,
    state: F,
    config: ConnectionConfig,
    shutdown: CancellationToken,
    drain: Option<Duration>,
    on_finish: Option<OnFinish>,
}

/// Serves `router` on `listener`, building per-connection state with `state`.
///
/// `state` is called once per accepted socket, with the address the listener reported --
/// `|_| S::default()` when the peer is of no interest.
///
/// The router is taken by value or as an [`Arc`] -- share one between two listeners by passing a
/// clone of the same `Arc`.
///
/// Nothing happens until the returned [`Server`] is awaited.
#[must_use = "a server does nothing until it is awaited"]
pub fn serve<L, S, F>(listener: L, router: impl Into<Arc<Router<S>>>, state: F) -> Server<L, S, F>
where
    L: Listener,
    S: Send + 'static,
    F: FnMut(&L::Addr) -> S + Send + 'static,
{
    Server {
        listener,
        router: router.into(),
        state,
        config: ConnectionConfig::default(),
        shutdown: CancellationToken::new(),
        drain: None,
        on_finish: None,
    }
}

impl<L, S, F> Server<L, S, F>
where
    L: Listener,
    S: Send + 'static,
    F: FnMut(&L::Addr) -> S + Send + 'static,
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

    /// Runs `on_finish` for every connection that ends, with the result it ended on.
    ///
    /// This is the reporting hook: what to log, and how loudly, is the caller's policy, so `serve`
    /// itself logs nothing about how a connection went. See
    /// `demo::server::log_completion` for the shape it expects.
    #[must_use]
    pub fn on_finish(
        mut self,
        on_finish: impl Fn(&Result<Completion>) + Send + Sync + 'static,
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
        // drain below graceful: cancelling the server stops accepts, not connections.
        let live = CancellationToken::new();
        let tasks = TaskTracker::new();

        loop {
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
            let (connection, _handle) = Connection::new(
                io,
                // One dispatcher per connection: two `Arc` clones, and the table it caches then
                // follows the version this connection negotiates.
                RouterDispatcher::new(Arc::clone(&self.router)),
                (self.state)(&addr),
                self.config.clone(),
                // Its own token, so a connection ending cancels itself and nothing else.
                live.child_token(),
            );

            let on_finish = self.on_finish.clone();
            tasks.spawn(async move {
                let result = connection.run().await;
                if let Some(on_finish) = on_finish {
                    on_finish(&result);
                }
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
                    live.cancel();
                    tasks.wait().await;
                }
            }
        }
    }
}

impl<L, S, F> IntoFuture for Server<L, S, F>
where
    L: Listener,
    S: Send + 'static,
    F: FnMut(&L::Addr) -> S + Send + 'static,
{
    type Output = ();
    type IntoFuture = BoxFuture<'static, ()>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
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
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::NotConnected
            | io::ErrorKind::Interrupted
    )
}
