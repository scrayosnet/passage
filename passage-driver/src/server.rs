//! The accept loop: one [`Connection`] per accepted socket.
//!
//! This is the piece that turns a [`Router`](crate::router::Router) -- static, built once, shared
//! -- into a running server. It is deliberately thin: accept, build the state for that peer, hand
//! both to a [`Connection`], and keep track of the task so shutdown can wait for it.
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, log_completion, router};
//! use passage_driver::server::Server;
//! use std::sync::Arc;
//! use tokio::net::TcpListener;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("0.0.0.0:25565").await?;
//!
//! Server::builder()
//!     .listener(listener)
//!     .dispatch(Arc::new(router()?))
//!     .state(|addr| Session {
//!         peer: Some(*addr),
//!         ..Session::default()
//!     })
//!     .on_finish(log_completion)
//!     .await;
//! # Ok(())
//! # }
//! ```
//!
//! [`serve`] is the same three arguments positionally, for the case where naming them adds nothing.
//!
//! # State is per connection, not shared
//!
//! This is where it differs from axum, whose `State` is one value shared by every request. Here
//! `S` is the connection's own state -- the hostname it asked for, the profile it authenticated
//! as, the keep-alive it owes an answer to -- so the server takes a *factory* rather than a value,
//! and calls it once per accepted socket with the peer's address. Anything genuinely shared belongs
//! in a captured [`Arc`] instead, either in that closure or in the handlers themselves.
//!
//! # What runs where
//!
//! Only the accept itself happens in the loop. Everything that touches a particular socket --
//! [`Listener::prepare`], the admission check, the state factory, the protocol -- runs on that
//! connection's own task, because all of them can wait on the peer and none of them may hold up
//! the next accept.
//!
//! # Each layer keeps its own books
//!
//! Three things here can turn a peer away, and none of them reports to the others:
//!
//! | Layer | Decides | Knows why |
//! |---|---|---|
//! | [`Listener::prepare`] | the preamble did not complete | the TLS error, the malformed header |
//! | [`Admit`] | this peer is not served right now | the bucket, the ban list, the rate |
//! | a handler | this *session* is refused | the packet, the phase, the session state |
//!
//! Each one holds, at the moment it decides, everything a metric about that decision could want --
//! so each one counts its own, and [`on_finish`](Server::on_finish) is left reporting exactly what
//! it is named for: connections that ran the protocol and then ended.
//!
//! The alternative is one hook told about every possible fate. It sounds like consolidation and is
//! the opposite: every layer has to flatten what it knows into a vocabulary this module invented for
//! it, the hook grows a match arm per layer, and the one place that ends up knowing everything is
//! the one place furthest from where any of it happened. The driver already refuses that trade one
//! level down -- there is no `Ending::Refused`, because the handler that refused a login knows it
//! did and writes the reason into its own state.
//!
//! # Writing your own loop
//!
//! Nothing here is privileged: it is a hundred lines over the public [`Connection`] API. Reach for
//! your own loop when you need something this does not offer -- a listener that is not one socket,
//! admission control that has to see the first packet, several routers behind one port.

use crate::conn::{Connection, ConnectionConfig, Dispatcher, Ending, Outcome};
use crate::error::Error;
use crate::packet::Phase;
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::FutureExt;
use futures::future::BoxFuture;
use std::any::Any;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::io;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
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
    /// The socket the protocol runs on, once the preamble is done.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// How a peer is identified. Handed to the state factory for every accepted connection.
    type Addr: fmt::Debug + Send + 'static;

    /// What [`accept`](Listener::accept) produces, before [`prepare`](Listener::prepare) has run.
    ///
    /// For a plain listener this is just the socket. It exists so that a preamble needing
    /// *configuration* is possible at all: `accept` takes `&self` and `prepare` does not, so
    /// anything the preamble needs from the listener -- a TLS acceptor, the set of proxies whose
    /// PROXY headers are trusted, a socket option to apply -- has to be handed over here, on the
    /// listener's own task, in the one place it can still be read.
    type Pending: Send + 'static;

    /// Accepts the next connection.
    ///
    /// Returning an error must be safe to retry: the loop logs it and calls `accept` again.
    ///
    /// The address comes back here rather than out of [`prepare`](Listener::prepare) so that a
    /// connection whose preamble *fails* can still be reported against the peer it came from.
    fn accept(&mut self) -> impl Future<Output = io::Result<(Self::Pending, Self::Addr)>> + Send;

    /// Prepares an accepted socket, **on that connection's own task**.
    ///
    /// This is where a preamble belongs: a PROXY protocol header, a TLS handshake, anything that
    /// has to read from the socket before the protocol starts. Doing it in
    /// [`accept`](Listener::accept) would be a mistake -- the loop awaits that one call, so a peer
    /// that connects and then says nothing would hold up every other connection to the server.
    ///
    /// `addr` is by `&mut` because overwriting it is the point of the PROXY case: what it is left
    /// as is what the state factory, the admission check and the report all see. Leaving it alone
    /// is the ordinary answer, and keeping ownership of it on the task means a failure here is
    /// still reportable.
    ///
    /// A listener that has no preamble writes `Ok(pending)`.
    fn prepare(
        pending: Self::Pending,
        addr: &mut Self::Addr,
    ) -> impl Future<Output = io::Result<Self::Io>> + Send;
}

impl Listener for TcpListener {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;
    type Pending = TcpStream;

    async fn accept(&mut self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        let (io, addr) = TcpListener::accept(self).await?;

        // Nagle holds a small write back waiting for more, and this protocol's writes are small and
        // final: a status response, a pong, a transfer. Batching them against a delayed ACK costs up
        // to ~40 ms on the one latency a player actually sees, the ping in the server list. There
        // is no switch for it because there is no case for the other setting -- a listener that
        // wants different socket options is a `Listener` impl, which is a dozen lines.
        //
        // Not fatal: the socket is still perfectly usable, it is just slower than it could be.
        if let Err(err) = io.set_nodelay(true) {
            trace!(cause = %err, ?addr, "could not disable Nagle on an accepted socket");
        }

        Ok((io, addr))
    }

    async fn prepare(
        pending: TcpStream,
        _addr: &mut std::net::SocketAddr,
    ) -> io::Result<TcpStream> {
        Ok(pending)
    }
}

/// Makes the [`Dispatcher`] for one accepted connection.
///
/// Implemented for `Arc<Router<S>>` -- in [`router`](crate::router), by the side that owns the
/// router -- which is the ordinary case: every connection gets a
/// [`RouterDispatcher`](crate::router::RouterDispatcher) over the same shared router. Implement it,
/// or use [`make_with`], to serve something that is not a router.
pub trait MakeDispatcher<S>: Send + 'static {
    /// The dispatcher this produces.
    type Dispatcher: Dispatcher<S> + Send + 'static;

    /// Makes one, for one connection.
    fn make(&self) -> Self::Dispatcher;
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

/// What a connection that ran the protocol reports to [`Server::on_finish`].
///
/// Only connections that *ran* arrive here. A peer the preamble rejected, or one
/// [`Admit`] turned away, is not a connection that ended -- it is a decision some other layer made,
/// and that layer is the one holding the reason. See [`Admit`].
///
/// The connection's own [`Outcome`] carries the state, the version and the phase it reached;
/// [`state`](Finished::state), [`version`](Finished::version) and [`phase`](Finished::phase) reach
/// through to it, and each answers [`None`] for exactly one reason -- the connection's task
/// panicked before it could say.
pub struct Finished<'a, S, A> {
    /// How long the connection lasted, from the accept to the last byte.
    ///
    /// Measured from the accept itself, not from the first poll of the connection's task, so the
    /// queueing delay a loaded server adds is inside the number rather than hidden from it.
    pub elapsed: Duration,

    /// The peer, as the listener reported it and [`Listener::prepare`] may have corrected it.
    /// Always known: it belongs to the task that reports, not to the one that ran the protocol, so
    /// even a panic cannot take it with it.
    pub addr: &'a A,

    /// How it ended, in the connection's own terms. A task that panicked is folded in here as an
    /// [`Ending::Failed`] with an internal error, so there is one thing to match on rather than a
    /// special case that only the server knows about.
    pub result: &'a std::result::Result<(), Ending>,

    outcome: Option<&'a Outcome<S>>,
}

impl<S, A> Finished<'_, S, A> {
    /// The state the connection was left in, unless its task panicked.
    #[must_use]
    pub fn state(&self) -> Option<&S> {
        self.outcome.map(|outcome| &outcome.state)
    }

    /// The protocol version it settled on, unless its task panicked.
    #[must_use]
    pub fn version(&self) -> Option<ProtocolVersion> {
        self.outcome.map(|outcome| outcome.version)
    }

    /// The phase it reached, unless its task panicked.
    #[must_use]
    pub fn phase(&self) -> Option<Phase> {
        self.outcome.map(|outcome| outcome.phase)
    }
}

/// Whether a peer is allowed in at all. See [`Server::on_accept`].
///
/// Implemented for every `Fn(&A) -> bool`, and for `()`, which admits everyone -- so a server that
/// sets no admission check pays no branch rather than an `Option` and a virtual call.
///
/// # It keeps its own books
///
/// `&self`, not `&mut self`, so an implementation is free to be a named type with state of its own
/// -- a token bucket per address, a counter, a clock. That is the point: **an admission check is
/// the only thing that knows it refused someone, and why**, so the count of refusals belongs on it
/// and not in a report somewhere downstream.
///
/// ```ignore
/// impl Admit<SocketAddr> for RateLimiter {
///     fn admit(&self, addr: &SocketAddr) -> bool {
///         let allowed = self.bucket(addr.ip()).take();
///         self.metrics.admitted_or_refused(allowed);   // counted here, where the reason is
///         allowed
///     }
/// }
/// ```
///
/// The server does not pass the refusal on to [`on_finish`](Server::on_finish), for the same reason
/// the driver has no `Completion::Refused`: a hook that has to hear about every possible fate grows
/// into one method that knows about all of them, and the layer that made the decision has to flatten
/// what it knows into a vocabulary the driver had to invent for it.
pub trait Admit<A>: Send + Sync + 'static {
    /// Whether to serve this peer.
    fn admit(&self, addr: &A) -> bool;
}

impl<A> Admit<A> for () {
    fn admit(&self, _addr: &A) -> bool {
        true
    }
}

impl<A, F> Admit<A> for F
where
    F: Fn(&A) -> bool + Send + Sync + 'static,
{
    fn admit(&self, addr: &A) -> bool {
        self(addr)
    }
}

/// What runs when a connection ends. See [`Server::on_finish`].
///
/// Implemented for every `Fn(&Finished<'_, S, A>)`, and for `()`, which reports nothing.
pub trait Report<S, A>: Send + Sync + 'static {
    /// Reports one finished connection.
    fn report(&self, finished: &Finished<'_, S, A>);
}

impl<S, A> Report<S, A> for () {
    fn report(&self, _finished: &Finished<'_, S, A>) {}
}

impl<S, A, F> Report<S, A> for F
where
    F: Fn(&Finished<'_, S, A>) + Send + Sync + 'static,
{
    fn report(&self, finished: &Finished<'_, S, A>) {
        self(finished);
    }
}

/// Accepts connections and runs one [`Connection`] per socket.
///
/// Built by [`Server::builder`] or by [`serve`], configured with the methods below, then awaited --
/// or run with [`run`](Server::run), which is the same thing without boxing the future.
///
/// # Three things it cannot do without
///
/// A listener, something to dispatch to, and a state factory. Each is a type parameter that starts
/// as `()` and is replaced by the setter that provides it, so [`run`](Server::run) exists only once
/// all three have been set -- forgetting one is a missing method rather than a runtime surprise.
/// Everything else has a default and can be set in any order.
///
/// # Why the hooks are type parameters
///
/// [`on_accept`](Server::on_accept) and [`on_finish`](Server::on_finish) are carried as their own
/// types rather than boxed, the way the state factory always was. Nobody writes a `Server` type
/// down -- it is built and awaited in one expression -- so the parameters cost nothing at any call
/// site, and the unset case is a `()` that compiles away instead of an `Option` tested per
/// connection. Contrast [`Router`](crate::router::Router), which *is* named -- in a factory's return
/// type, in an application's struct -- and therefore has to erase its handlers.
#[must_use = "a server does nothing until it is awaited"]
pub struct Server<L, F, M, A = (), R = ()> {
    listener: L,
    state: F,
    make: M,
    config: ConnectionConfig,
    shutdown: CancellationToken,
    drain: Option<Duration>,
    limit: Option<Arc<Semaphore>>,
    on_accept: A,
    on_finish: R,
}

impl Server<(), (), ()> {
    /// Starts building a server.
    pub fn builder() -> Self {
        Server {
            listener: (),
            state: (),
            make: (),
            config: ConnectionConfig::default(),
            shutdown: CancellationToken::new(),
            drain: None,
            limit: None,
            on_accept: (),
            on_finish: (),
        }
    }
}

/// Serves `make`'s dispatchers on `listener`, building per-connection state with `state`.
///
/// The positional form of [`Server::builder`]'s three required setters, for the case where naming
/// them adds nothing: `make` is usually an `Arc<Router<S>>`, and `state` is called once per accepted
/// socket with the address the listener reported -- `|_| S::default()` when the peer is of no
/// interest.
///
/// Nothing happens until the returned [`Server`] is awaited.
pub fn serve<L, S, F, M>(listener: L, make: M, state: F) -> Server<L, F, M>
where
    L: Listener,
    F: Fn(&L::Addr) -> S,
{
    Server::builder()
        .listener(listener)
        .dispatch(make)
        .state(state)
}

// Split across four impl blocks by what each setter needs to know. It is not decoration: a setter
// that takes a closure has to be able to *type* it, and `|addr| ...` can only be inferred where the
// bound naming `L::Addr` is already in scope. So the three that take closures come with their
// bounds, which in turn fixes the order the chain is written in -- listener, then state, then the
// hooks that report on it. The rest constrain nothing and compose freely.
impl<L, F, M, A, R> Server<L, F, M, A, R> {
    /// Sets where connections come from.
    pub fn listener<L2>(self, listener: L2) -> Server<L2, F, M, A, R> {
        Server {
            listener,
            state: self.state,
            make: self.make,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
            on_accept: self.on_accept,
            on_finish: self.on_finish,
        }
    }

    /// Sets what every connection dispatches to -- usually an `Arc<Router<S>>`.
    pub fn dispatch<M2>(self, make: M2) -> Server<L, F, M2, A, R> {
        Server {
            listener: self.listener,
            state: self.state,
            make,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
            on_accept: self.on_accept,
            on_finish: self.on_finish,
        }
    }

    /// Sets the whole configuration every connection is created with.
    ///
    /// The individual knobs below set one field of it each; this replaces all of them at once.
    pub fn config(mut self, config: ConnectionConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the decoding limits every connection is created with.
    pub fn limits(mut self, limits: Limits) -> Self {
        self.config.limits = limits;
        self
    }

    /// Sets how often the tick handler runs. See [`ConnectionConfig::tick_interval`].
    pub fn tick_interval(mut self, interval: Duration) -> Self {
        self.config.tick_interval = Some(interval);
        self
    }

    /// Caps how long one connection may last. See [`ConnectionConfig::max_lifetime`].
    ///
    /// `None` removes the cap, which a server reaching [`Phase::Play`] has to say explicitly.
    pub fn max_lifetime(mut self, after: Option<Duration>) -> Self {
        self.config.max_lifetime = after;
        self
    }

    /// Caps how long a connection may spend writing what it owes once it is already ending.
    /// See [`ConnectionConfig::close_timeout`].
    pub fn close_timeout(mut self, after: Option<Duration>) -> Self {
        self.config.close_timeout = after;
        self
    }

    /// Sets the protocol version a connection starts on, before its handshake.
    pub fn initial_version(mut self, version: ProtocolVersion) -> Self {
        self.config.initial_version = version;
        self
    }

    /// Sets the phase a connection starts in.
    pub fn initial_phase(mut self, phase: Phase) -> Self {
        self.config.initial_phase = phase;
        self
    }

    /// Stops accepting when `shutdown` is cancelled, then waits for live connections to finish.
    ///
    /// Waiting is the point: a connection that is mid-transfer gets to send its transfer packet.
    /// What bounds that wait is [`ConnectionConfig::max_lifetime`], or a
    /// [`drain_timeout`](Server::drain_timeout) if you would rather cap it here.
    pub fn graceful_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Cancels connections that are still live this long after the shutdown signal.
    ///
    /// Without it, the server waits for every connection to end on its own.
    pub fn drain_timeout(mut self, after: Duration) -> Self {
        self.drain = Some(after);
        self
    }

    /// Caps how many connections may be live at once.
    ///
    /// The slot is taken *before* the accept, so a server at its limit leaves the next connection
    /// in the kernel's backlog instead of accepting it only to drop it -- which is the difference
    /// between backpressure and a refusal the peer cannot distinguish from an outage.
    pub fn max_connections(mut self, limit: usize) -> Self {
        self.limit = Some(Arc::new(Semaphore::new(limit)));
        self
    }
}

impl<L: Listener, F, M, A, R> Server<L, F, M, A, R> {
    /// Sets the factory that builds one connection's state, called once per accepted socket.
    ///
    /// Needs the listener to have been set, because what the factory is handed is `L::Addr`.
    pub fn state<S, F2>(self, state: F2) -> Server<L, F2, M, A, R>
    where
        F2: Fn(&L::Addr) -> S,
    {
        Server {
            listener: self.listener,
            state,
            make: self.make,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
            on_accept: self.on_accept,
            on_finish: self.on_finish,
        }
    }

    /// Decides whether a peer is served at all, once its real address is known.
    ///
    /// This is where per-IP rate limiting goes. It runs on the connection's own task, after
    /// [`Listener::prepare`], so it sees the address a PROXY header reported rather than the
    /// immediate peer's -- and a slow preamble cannot hold up the accept loop. Returning `false`
    /// closes the socket without a byte of protocol, and reports nothing to
    /// [`on_finish`](Server::on_finish): counting refusals is the limiter's own job, and it is
    /// holding the reason. See [`Admit`].
    pub fn on_accept<A2: Admit<L::Addr>>(self, on_accept: A2) -> Server<L, F, M, A2, R> {
        Server {
            listener: self.listener,
            state: self.state,
            make: self.make,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
            on_accept,
            on_finish: self.on_finish,
        }
    }
}

impl<L, S, F, M, A, R> Server<L, F, M, A, R>
where
    L: Listener,
    F: Fn(&L::Addr) -> S,
{
    /// Runs `on_finish` for every connection that ends, with what it ended on.
    ///
    /// This is the reporting hook: what to log, and how loudly, is the caller's policy, so the
    /// server itself logs nothing about how a connection went. It runs for every connection that
    /// ran the protocol, including one whose task *panicked* -- otherwise that would be the one
    /// outcome that never reached the metrics. It does **not** run for a peer the preamble or
    /// [`Admit`] turned away, which never became a connection and whose reason lives with the layer
    /// that decided. See `demo::server::log_completion` for the shape it expects.
    ///
    /// Needs the listener and the state factory to have been set: what it is handed is built out
    /// of both.
    pub fn on_finish<R2: Report<S, L::Addr>>(self, on_finish: R2) -> Server<L, F, M, A, R2> {
        Server {
            listener: self.listener,
            state: self.state,
            make: self.make,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
            on_accept: self.on_accept,
            on_finish,
        }
    }
}

impl<L, S, F, M, A, R> Server<L, F, M, A, R>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Admit<L::Addr>,
    R: Report<S, L::Addr>,
{
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

        // Allocated once rather than per connection: what a connection task needs from the server
        // is the same for every connection, and three separate `Arc`s would be three clones per
        // accept to say so.
        let hooks = Arc::new(Hooks {
            state: self.state,
            on_accept: self.on_accept,
            on_finish: self.on_finish,
            config: self.config,
        });

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

            // The second shutdown arm is not a copy of the first: they guard different awaits, and
            // a server that is already at its limit would otherwise sit in `acquire_owned` with
            // nothing watching the token.
            let accepted = tokio::select! {
                biased;

                () = self.shutdown.cancelled() => break,

                accepted = self.listener.accept() => accepted,
            };

            let (pending, addr) = match accepted {
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
            // Started here rather than on the connection's own task, so that the time a loaded
            // server spends between `spawn` and the first poll is inside the duration it reports.
            let started = Instant::now();
            // One dispatcher per connection: for a router, one `Arc` clone and a table index that
            // then follows the version this connection negotiates.
            let dispatcher = self.make.make();
            // Its own token, so a connection ending cancels itself and nothing else.
            let shutdown = live.0.child_token();

            tasks.spawn(connection::<L, S, F, M::Dispatcher, A, R>(
                Arc::clone(&hooks),
                pending,
                addr,
                dispatcher,
                shutdown,
                started,
                permit,
            ));
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

impl<L, S, F, M, A, R> IntoFuture for Server<L, F, M, A, R>
where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Admit<L::Addr>,
    R: Report<S, L::Addr>,
{
    type Output = ();
    // Boxed because there is nothing else to write here: the future is an `async fn` body, whose
    // type has no name, and naming it would need `impl Future` in an associated type position --
    // which is still unstable. `run` is the same future, unboxed, for anyone who minds.
    type IntoFuture = BoxFuture<'static, ()>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

/// What every connection task needs from the server, shared by all of them.
struct Hooks<F, A, R> {
    state: F,
    on_accept: A,
    on_finish: R,
    config: ConnectionConfig,
}

/// One connection, from the socket the loop accepted to the report it ends with.
///
/// Separate from the accept loop because it is the half that grows -- everything a connection can
/// do to a peer happens here, and none of it concerns accepting -- and because it can then be read,
/// and reasoned about, without a listener anywhere in sight.
async fn connection<L, S, F, D, A, R>(
    hooks: Arc<Hooks<F, A, R>>,
    pending: L::Pending,
    mut addr: L::Addr,
    dispatcher: D,
    shutdown: CancellationToken,
    started: Instant,
    permit: Option<OwnedSemaphorePermit>,
) where
    L: Listener,
    S: Send + 'static,
    F: Fn(&L::Addr) -> S + Send + Sync + 'static,
    D: Dispatcher<S> + Send + 'static,
    A: Admit<L::Addr>,
    R: Report<S, L::Addr>,
{
    // Held for exactly as long as the connection is live, and released by dropping.
    let _permit = permit;

    // Neither of the two ways out below is reported. A preamble that failed and an admission check
    // that said no are decisions made *by* the listener and *by* the check, each of which still has
    // in hand the thing it decided on -- the TLS error, the bucket that was empty. Handing a
    // flattened version of that to a hook further down would make the hook the only place that knows
    // anything, which is the shape this crate avoids everywhere else: the handler that refuses a
    // login records the reason in its own state too.
    let io = match L::prepare(pending, &mut addr).await {
        Ok(io) => io,
        Err(err) => {
            debug!(cause = %err, ?addr, "failed to prepare an accepted socket");
            return;
        }
    };

    if !hooks.on_accept.admit(&addr) {
        debug!(?addr, "refused a connection before the protocol started");
        return;
    }

    let (connection, _handle) = Connection::builder(io, dispatcher, (hooks.state)(&addr))
        .config(hooks.config)
        .shutdown(shutdown)
        .build();

    // Caught rather than spawned onto a task of its own, so that a panic in a handler is something
    // we can *see* without paying a second `tokio::spawn` for every connection that never panics.
    // `AssertUnwindSafe` is a claim about what is observed after a panic, and here that is only
    // *that* it panicked: the state is dropped, never read. Under `panic = "abort"` neither this
    // nor a second task does anything.
    let outcome = AssertUnwindSafe(connection.run()).catch_unwind().await;

    // A panic is folded into an `Ending` rather than given a shape of its own: it is still a
    // connection that ran and then stopped, so it is still one thing to match on.
    let panicked;
    let (outcome, result) = match &outcome {
        Ok(outcome) => (Some(outcome), &outcome.result),
        Err(payload) => {
            panicked = Err(Ending::Failed(Error::internal(
                "panic",
                panic_message(&**payload),
            )));
            (None, &panicked)
        }
    };

    hooks.on_finish.report(&Finished {
        elapsed: started.elapsed(),
        addr: &addr,
        result,
        outcome,
    });
}

/// Whatever a panicking handler said, if it said anything printable.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a handler panicked".to_owned()
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
