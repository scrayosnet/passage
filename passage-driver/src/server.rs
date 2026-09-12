//! The accept loop: one [`Connection`] per accepted socket.
//!
//! This is the piece that turns a [`Router`](crate::router::Router) -- static, built once, shared
//! -- into a running server. It is deliberately thin: accept, build the state for that peer, hand
//! both to a [`Connection`], and keep track of the task so shutdown can wait for it.
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, router};
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
//! # Everything between the accept and the protocol is a layer
//!
//! A socket usually needs something done to it before the protocol starts: a PROXY header read, a
//! TLS handshake completed, a rate limiter consulted. All of those are the same shape -- take the
//! socket and the address, hand back a socket and an address, or refuse -- so there is one mechanism
//! for them, [`Layer`], and the accept loop knows about none of them individually:
//!
//! ```ignore
//! Server::builder()
//!     .listener(listener)
//!     .layer(ProxyProtocol::new(trusted))   // rewrites the address
//!     .layer(Tls::new(acceptor))            // changes the socket type
//!     .layer(rate_limiter)                  // refuses, and counts its own refusals
//!     .dispatch(router)
//!     .state(session)
//! ```
//!
//! Layers run in the order they are written, each seeing what the one before produced, all of them
//! on the connection's own task -- so a peer that connects and then says nothing holds up nobody
//! else. Refusing is [`None`], with no reason attached, because the layer that refused is the one
//! holding the reason; see [`Layer`].
//!
//! This module ships no layers at all. A PROXY implementation belongs next to the PROXY parser, and
//! reaches the builder as an extension trait over [`Server`] -- which is why nothing here mentions
//! proxies, TLS or rate limits.
//!
//! # What runs where
//!
//! Only the accept itself happens in the loop. Everything that touches a particular socket -- the
//! layers, the state factory, the protocol -- runs on that connection's own task, because all of
//! them can wait on the peer and none of them may hold up the next accept.
//!
//! # What it records
//!
//! Every connection runs inside a `connection` span carrying its peer, and ends with one event
//! saying how -- at `warn` if the cause was ours, at `debug` otherwise. That is the whole of the
//! driver's reporting, and it is deliberately only what the *driver* knows: how long it lasted,
//! which [`Ending`] it had, the version and phase it reached.
//!
//! There is no hook for the rest. Facts about the application -- the hostname asked for, the profile
//! verified, the backend chosen -- belong to the handler that established them, which can log them
//! the moment they are true and inside this span. A hook that had to be told everything would mean
//! every layer flattening what it knows into a vocabulary invented here, and one central reporter
//! knowing things it is in the worst position to describe.

use crate::conn::{Connection, ConnectionConfig, Dispatcher, Ending};
use crate::error::{Class, Error};
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
use tracing::{Instrument, Span, debug, field, info_span, trace, warn};

/// How long to wait before accepting again after an error that is not the peer's doing.
///
/// Running out of file descriptors is the case this exists for: it resolves itself as connections
/// close, so the loop must neither give up nor spin.
const ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

/// A source of connections.
///
/// Implemented for [`TcpListener`]. Implement it yourself to serve something else -- a Unix socket,
/// a socket pair in a test, or a transport of your own.
///
/// It is only a source. Anything that has to *happen* to an accepted socket before the protocol
/// starts -- a preamble to read, a handshake to complete, a check to pass -- is a [`Layer`], not a
/// listener's business.
pub trait Listener: Send + 'static {
    /// The socket this produces.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// How a peer is identified. A [`Layer`] may replace the value -- that is what a PROXY header
    /// does -- so what reaches the state factory is what the last layer left behind.
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
        let (io, addr) = TcpListener::accept(self).await?;

        // Nagle holds a small write back waiting for more, and this protocol's writes are small and
        // final: a status response, a pong, a transfer. Batching them against a delayed ACK costs up
        // to ~40 ms on the one latency a player actually sees, the ping in the server list. A
        // listener that wants different socket options is a `Listener` impl, which is now four
        // lines.
        //
        // Not fatal: the socket is still perfectly usable, it is just slower than it could be.
        if let Err(err) = io.set_nodelay(true) {
            trace!(cause = %err, ?addr, "could not disable Nagle on an accepted socket");
        }

        Ok((io, addr))
    }
}

/// Something done to an accepted socket before the protocol starts.
///
/// One shape covers every case: take the socket and the address, hand back a socket and an address,
/// or refuse. A PROXY layer replaces the address; a TLS layer replaces the socket *type*, through
/// [`Io`](Layer::Io); a rate limiter replaces neither and sometimes says no.
///
/// Set them with [`Server::layer`]. They run in the order written, each seeing what the one before
/// produced, all on the connection's own task.
///
/// # Refusing says nothing, on purpose
///
/// [`None`] carries no reason, because there is nowhere better for the reason to be than in the
/// layer that decided. It is holding the expired bucket, the untrusted source address, the
/// certificate that failed to verify -- everything a metric or a log line about that decision could
/// want. `&self` is what lets it keep books:
///
/// ```ignore
/// impl<Io: Send + 'static> Layer<Io, SocketAddr> for RateLimiter {
///     type Io = Io;
///
///     async fn admit(&self, io: Io, addr: SocketAddr) -> Option<(Io, SocketAddr)> {
///         if self.bucket(addr.ip()).take() {
///             return Some((io, addr));
///         }
///         self.refusals.fetch_add(1, Ordering::Relaxed);   // counted here, where the reason is
///         None
///     }
/// }
/// ```
///
/// A `Result` here would only let a layer summarise what it already knows into a vocabulary this
/// module had to invent, and hand it to something further away.
///
/// # Implementations that come for free
///
/// * `()` -- no layers. The identity, and what a server that sets none is built with.
/// * any `Fn(&Addr) -> bool` -- the admission case, for when a closure is enough.
/// * [`Stack`] -- two layers as one, which is how [`Server::layer`] composes them.
pub trait Layer<Io, Addr>: Send + Sync + 'static {
    /// The socket the next layer sees. Usually `Io` itself; a layer that upgrades the transport
    /// changes it.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// Admits this peer, having done whatever this layer does to its socket first.
    fn admit(&self, io: Io, addr: Addr) -> impl Future<Output = Option<(Self::Io, Addr)>> + Send;
}

/// No layers: every peer is admitted with the socket the listener produced.
impl<Io, Addr> Layer<Io, Addr> for ()
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Addr: Send,
{
    type Io = Io;

    async fn admit(&self, io: Io, addr: Addr) -> Option<(Io, Addr)> {
        Some((io, addr))
    }
}

/// The admission case: a predicate over the address, for when a closure is enough and there is
/// nothing to count.
impl<Io, Addr, F> Layer<Io, Addr> for F
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Addr: Send + Sync,
    F: Fn(&Addr) -> bool + Send + Sync + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, addr: Addr) -> Option<(Io, Addr)> {
        self(&addr).then_some((io, addr))
    }
}

/// Two layers as one: `A` first, then `B` over what `A` produced.
///
/// Built by [`Server::layer`]; there is no reason to name it.
pub struct Stack<A, B>(A, B);

impl<Io, Addr, A, B> Layer<Io, Addr> for Stack<A, B>
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Addr: Send + 'static,
    A: Layer<Io, Addr>,
    B: Layer<A::Io, Addr>,
{
    type Io = B::Io;

    async fn admit(&self, io: Io, addr: Addr) -> Option<(B::Io, Addr)> {
        let (io, addr) = self.0.admit(io, addr).await?;
        self.1.admit(io, addr).await
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
/// Everything else has a default and can be set in any order, except that
/// [`listener`](Server::listener) comes first: the setters that take a closure need `L::Addr` in
/// scope to be able to type it.
///
/// # Why the parts are type parameters
///
/// The state factory and the layer stack are carried as their own types rather than boxed. Nobody
/// writes a `Server` type down -- it is built and awaited in one expression -- so the parameters
/// cost nothing at any call site, the unset case is a `()` that compiles away, and a layer that
/// changes the socket type can say so. Contrast [`Router`](crate::router::Router), which *is* named
/// -- in a factory's return type, in an application's struct -- and therefore has to erase its
/// handlers.
#[must_use = "a server does nothing until it is awaited"]
pub struct Server<L, F, M, A = ()> {
    listener: L,
    state: F,
    make: M,
    layers: A,
    config: ConnectionConfig,
    shutdown: CancellationToken,
    drain: Option<Duration>,
    limit: Option<Arc<Semaphore>>,
}

impl Server<(), (), ()> {
    /// Starts building a server.
    pub fn builder() -> Self {
        Server {
            listener: (),
            state: (),
            make: (),
            layers: (),
            config: ConnectionConfig::default(),
            shutdown: CancellationToken::new(),
            drain: None,
            limit: None,
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

// Split by what each setter needs to know. It is not decoration: a setter that takes a closure has
// to be able to *type* it, and `|addr| ...` can only be inferred where the bound naming `L::Addr` is
// already in scope. That is what fixes `listener` to the front of the chain; everything below
// constrains nothing and composes freely.
impl<L, F, M, A> Server<L, F, M, A> {
    /// Sets where connections come from.
    pub fn listener<L2>(self, listener: L2) -> Server<L2, F, M, A> {
        Server {
            listener,
            state: self.state,
            make: self.make,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }

    /// Sets what every connection dispatches to -- usually an `Arc<Router<S>>`.
    pub fn dispatch<M2>(self, make: M2) -> Server<L, F, M2, A> {
        Server {
            listener: self.listener,
            state: self.state,
            make,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
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

impl<L: Listener, F, M, A: Layer<L::Io, L::Addr>> Server<L, F, M, A> {
    /// Adds a [`Layer`], after any already added.
    ///
    /// It sees the socket and address the previous layer produced, and what it produces is what the
    /// next one sees -- so the order here is the order things happen to the socket. A PROXY header
    /// is read before a TLS handshake because it is written that way, not because either knows
    /// about the other.
    ///
    /// All of them run on the connection's own task, so a layer that waits on the peer delays
    /// nothing but that peer.
    pub fn layer<A2: Layer<A::Io, L::Addr>>(self, layer: A2) -> Server<L, F, M, Stack<A, A2>> {
        Server {
            listener: self.listener,
            state: self.state,
            make: self.make,
            layers: Stack(self.layers, layer),
            config: self.config,
            shutdown: self.shutdown,
            drain: self.drain,
            limit: self.limit,
        }
    }
}

impl<L: Listener, F, M, A> Server<L, F, M, A> {
    /// Sets the factory that builds one connection's state, called once per accepted socket.
    ///
    /// It is handed the address the layers left behind, not the one the listener first reported.
    pub fn state<S, F2>(self, state: F2) -> Server<L, F2, M, A>
    where
        F2: Fn(&L::Addr) -> S,
    {
        Server {
            listener: self.listener,
            state,
            make: self.make,
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
        // is the same for every connection, and separate `Arc`s would be a clone each per accept.
        let shared = Arc::new(Shared {
            state: self.state,
            layers: self.layers,
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
            // Started here rather than on the connection's own task, so that the time a loaded
            // server spends between `spawn` and the first poll is inside the duration it reports.
            let started = Instant::now();
            // One dispatcher per connection: for a router, one `Arc` clone and a table index that
            // then follows the version this connection negotiates.
            let dispatcher = self.make.make();
            // Its own token, so a connection ending cancels itself and nothing else.
            let shutdown = live.0.child_token();

            // `peer` is filled in once the layers have run, because until then we do not know who
            // this is: a PROXY header may still replace it. Everything a layer logs is already
            // inside the span, which is the point of opening it here.
            let span = info_span!("connection", peer = field::Empty);

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
        Box::pin(self.run())
    }
}

/// What every connection task needs from the server, shared by all of them.
struct Shared<F, A> {
    state: F,
    layers: A,
    config: ConnectionConfig,
}

/// One connection, from the socket the loop accepted to the record it ends with.
///
/// Separate from the accept loop because it is the half that grows -- everything that happens to a
/// peer happens here, and none of it concerns accepting -- and because it can then be read, and
/// reasoned about, without a listener anywhere in sight.
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
    // Held for exactly as long as the connection is live, and released by dropping.
    let _permit = permit;

    // Nothing is logged about a refusal here, and that is the design: whichever layer said no is
    // holding the reason, and has already done whatever it wanted with it.
    let Some((io, addr)) = shared.layers.admit(io, addr).await else {
        return;
    };
    Span::current().record("peer", field::debug(&addr));

    let (connection, _handle) = Connection::builder(io, dispatcher, (shared.state)(&addr))
        .config(shared.config)
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
    let (reached, result) = match &outcome {
        Ok(outcome) => (Some((outcome.version, outcome.phase)), &outcome.result),
        Err(payload) => {
            panicked = Err(Ending::Failed(Error::internal(
                "panic",
                panic_message(&**payload),
            )));
            (None, &panicked)
        }
    };
    log_ending(started.elapsed(), result, reached);
}

/// The one thing the driver records about a finished connection, at the level it deserves.
///
/// Only what the driver itself knows. Everything else a report might have wanted -- which host was
/// asked for, which profile was verified, which backend was chosen -- is known earlier and more
/// precisely by the handler that established it, and belongs in a line that handler writes inside
/// this connection's span.
fn log_ending(
    elapsed: Duration,
    result: &std::result::Result<(), Ending>,
    reached: Option<(ProtocolVersion, Phase)>,
) {
    let version = reached.map(|(version, _)| version);
    let phase = reached.map(|(_, phase)| phase);

    let Err(ending) = result else {
        debug!(?elapsed, ?version, ?phase, "connection closed");
        return;
    };

    match ending.error() {
        // Our bug, or something that should not have been possible: the one case that is not
        // routine, and the one an operator has to see.
        Some(err) if matches!(err.class(), Class::Internal) => warn!(
            ?elapsed,
            ?version,
            ?phase,
            kind = err.label(),
            cause = %err,
            "connection failed",
        ),
        // A hangup, a timeout, a shutdown, a peer that sent nonsense. All ordinary weather, and a
        // scanner that took its MOTD and left looks exactly like the first of them.
        _ => debug!(
            ?elapsed,
            ?version,
            ?phase,
            ending = ending.label(),
            "connection ended",
        ),
    }
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
