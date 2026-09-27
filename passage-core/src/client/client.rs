use crate::client::connector::Connector;
use crate::client::error::{ClientError, Result};
use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::{Connection, MakeDispatcher, Options as ConnectionConfig, Outcome};
use crate::router::{Layer, Stack};
use crate::wire::Options as WireOptions;
use futures::future::BoxFuture;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, Span, debug, field, info_span};

/// The [`Client`] opens a connection with the configured connector, passes it through the configured
/// layers, and creates a new connection from it.
#[must_use = "a client does nothing until it is awaited"]
pub struct Client<C = (), F = (), M = (), A = ()> {
    /// The connector that opens the socket.
    connector: C,

    /// The state factory from which the state is build and passed to the connection.
    state: F,

    /// The dispatcher factory from which the dispatcher is build and passed to the connection.
    dispatcher: M,

    /// The layers to pass the connection through.
    layers: A,

    /// The configuration to pass to the connection.
    config: ConnectionConfig,

    /// The shutdown signal. The connection gets a child token.
    shutdown: CancellationToken,
}

impl<C: Connector> Client<C, (), (), ()> {
    /// Starts building a client that connects with `connector`.
    pub fn new(connector: C) -> Self {
        Client {
            connector,
            state: (),
            dispatcher: (),
            layers: (),
            config: ConnectionConfig::default(),
            shutdown: CancellationToken::new(),
        }
    }
}

impl<C: Connector, F, M, A: Layer<C::Io, C::Addr>> Client<C, F, M, A> {
    /// Sets the client connector.
    pub fn connector<C2>(self, connector: C2) -> Client<C2, F, M, A> {
        Client {
            connector,
            state: self.state,
            dispatcher: self.dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
        }
    }

    /// Sets the client dispatch factory.
    ///
    /// A client uses one connection, so the factory is called once. It is a factory anyway because
    /// that is what [`Arc<Router<S>>`](crate::router::Router) implements, and a client that could
    /// not be handed a router would miss the point.
    pub fn dispatch<M2>(self, dispatcher: M2) -> Client<C, F, M2, A> {
        Client {
            connector: self.connector,
            state: self.state,
            dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
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

    /// Sets the shutdown token for the client.
    pub fn graceful_shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// Adds a new layer to the client, stacked behind the existing ones.
    pub fn layer<A2: Layer<A::Io, C::Addr>>(self, layer: A2) -> Client<C, F, M, Stack<A, A2>> {
        Client {
            connector: self.connector,
            state: self.state,
            dispatcher: self.dispatcher,
            layers: Stack::new(self.layers, layer),
            config: self.config,
            shutdown: self.shutdown,
        }
    }

    /// Sets the state factory for the client.
    pub fn state<S, F2>(self, state: F2) -> Client<C, F2, M, A>
    where
        F2: Fn(&C::Addr) -> S,
    {
        Client {
            connector: self.connector,
            state,
            dispatcher: self.dispatcher,
            layers: self.layers,
            config: self.config,
            shutdown: self.shutdown,
        }
    }
}

impl<C, S, F, M, A> Client<C, F, M, A>
where
    C: Connector,
    S: Send + 'static,
    F: Fn(&C::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Layer<C::Io, C::Addr>,
{
    /// Opens the connection and drives it to completion.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError`] if the connector could not open a socket, or a layer rejected the
    /// one it opened. Either way, the protocol never started, so there is no outcome to report.
    pub async fn connect(mut self) -> Result<Outcome<S>> {
        let (io, addr) = self
            .connector
            .connect()
            .await
            .map_err(ClientError::Connect)?;
        debug!(address = ?addr, "opened a connection");

        let span = info_span!("connection", peer = field::Empty);
        async move {
            // Apply the layer stack. If the layer stack is empty, then the connection is just a raw
            // I/O. Afterward, the final peer address is recorded.
            let Some((io, addr)) = self.layers.admit(io, addr).await else {
                return Err(ClientError::Rejected);
            };
            Span::current().record("peer", field::debug(&addr));

            // The connection runs on a child token, because it cancels its own on the way out and
            // the caller's token is not ours to spend.
            let connection = Connection::<_, (), ()>::builder(io)
                .dispatcher(self.dispatcher.make())
                .state((self.state)(&addr))
                .config(self.config)
                .shutdown(self.shutdown.child_token())
                .build();

            let outcome = connection.run().await;
            match &outcome.error {
                None => {
                    debug!(version = ?outcome.version, phase = ?outcome.phase, "connection closed")
                }
                Some(err) => debug!(
                    version = ?outcome.version,
                    phase = ?outcome.phase,
                    reason = err.reason(),
                    "connection ended"
                ),
            }
            Ok(outcome)
        }
        .instrument(span)
        .await
    }
}

impl<C, S, F, M, A> IntoFuture for Client<C, F, M, A>
where
    C: Connector,
    S: Send + 'static,
    F: Fn(&C::Addr) -> S + Send + Sync + 'static,
    M: MakeDispatcher<S>,
    A: Layer<C::Io, C::Addr>,
{
    type Output = Result<Outcome<S>>;
    // Boxed for the reason `Server`'s is: the future is an `async fn` body, whose type has no name.
    // `connect` is the same future, unboxed, for anyone who minds.
    type IntoFuture = BoxFuture<'static, Result<Outcome<S>>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.connect())
    }
}
