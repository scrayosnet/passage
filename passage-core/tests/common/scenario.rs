//! Two routers, one socket pair, and both endings.

use super::notes::Notes;
use super::raw::RawClient;
use passage_core::client::{Client, Connected};
use passage_core::connection::{Options, Outcome};
use passage_core::router::Router;
use passage_core::{Phase, ProtocolVersion};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Which end of the socket pair a connection is, which is all an in-process peer has for an
/// address. A named type rather than a string, so that the client's type can be written down.
#[derive(Copy, Clone, Debug)]
pub enum Side {
    /// The side that answers.
    Server,
    /// The side that dialled.
    Client,
}

/// How long a scenario may take before the test is called hung.
///
/// Far above any deadline a test configures, because a paused clock jumps to the *earliest* pending
/// deadline: this one only fires when nothing else is waiting for anything, which is the definition
/// of the hang it exists to catch.
const HUNG: Duration = Duration::from_secs(300);

/// The state factory, as a function so that the client type can be named.
fn notes(_side: &Side) -> Notes {
    Notes::default()
}

/// A client built for a test: one socket, one router, notes for state.
pub type TestClient =
    Client<Connected<DuplexStream, Side>, fn(&Side) -> Notes, Arc<Router<Notes>>, ()>;

/// Builds a client that speaks `router` over `io`.
///
/// This is the seam every test shares. What a test still has to say is the version it starts at and
/// the deadlines it cares about; everything else has a default that is right for a test.
pub fn client(io: DuplexStream, router: Router<Notes>) -> TestClient {
    Client::new(Connected::new(io, Side::Client))
        .state(notes as fn(&Side) -> Notes)
        .dispatch(Arc::new(router))
}

/// Runs a server router and a client router against each other over an in-process socket pair.
///
/// A test says what the two sides route and nothing else:
///
/// ```ignore
/// let meeting = Scenario::new(server_router, client_router)
///     .version(versions::V26_2)
///     .run()
///     .await;
/// meeting.expect_clean();
/// assert!(meeting.server.state.saw("handshake"));
/// ```
pub struct Scenario {
    server: Router<Notes>,
    client: Router<Notes>,
    server_config: Options,
    client_config: Options,
    shutdown: CancellationToken,
    buffer: usize,
    timeout: Duration,
}

impl Scenario {
    /// A scenario in which `server` answers and `client` speaks first.
    pub fn new(server: Router<Notes>, client: Router<Notes>) -> Self {
        Self {
            server,
            client,
            server_config: Options::default(),
            client_config: Options::default(),
            shutdown: CancellationToken::new(),
            buffer: 4096,
            timeout: HUNG,
        }
    }

    /// The version the *client* starts at.
    ///
    /// Only the client: a server learns the version from the handshake, which is the asymmetry the
    /// whole protocol is built around. A scenario that set both would never exercise it.
    pub fn version(mut self, version: ProtocolVersion) -> Self {
        self.client_config.initial_version = version;
        self
    }

    /// The phase both sides start in, for a test that does not want to walk there first.
    pub fn phase(mut self, phase: Phase) -> Self {
        self.server_config.initial_phase = phase;
        self.client_config.initial_phase = phase;
        self
    }

    /// Adjusts the server's connection options.
    pub fn server_config(mut self, with: impl FnOnce(&mut Options)) -> Self {
        with(&mut self.server_config);
        self
    }

    /// Adjusts the client's connection options.
    pub fn client_config(mut self, with: impl FnOnce(&mut Options)) -> Self {
        with(&mut self.client_config);
        self
    }

    /// Runs both sides under `shutdown`, so cancelling it ends the scenario.
    pub fn shutdown(mut self, shutdown: CancellationToken) -> Self {
        self.shutdown = shutdown;
        self
    }

    /// How many bytes fit in the socket pair before a write blocks.
    ///
    /// A small buffer is a peer that stopped reading, which is the cheapest attack on a protocol
    /// server and the reason every write is guarded.
    pub fn buffer(mut self, bytes: usize) -> Self {
        self.buffer = bytes;
        self
    }

    /// How long the scenario may take before the test is called hung.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Runs both sides to completion and reports how each of them ended.
    pub async fn run(self) -> Meeting {
        let (server_io, client_io) = tokio::io::duplex(self.buffer);

        let server = Client::new(Connected::new(server_io, Side::Server))
            .state(notes as fn(&Side) -> Notes)
            .dispatch(Arc::new(self.server))
            .config(self.server_config)
            .graceful_shutdown(self.shutdown.clone())
            .connect();
        let client = client(client_io, self.client)
            .config(self.client_config)
            .graceful_shutdown(self.shutdown.clone())
            .connect();

        // Both on one task: a duplex pair is only ever driven by whichever side is being polled, so
        // there is nothing to gain from a second thread and a test failure keeps its backtrace.
        let both = async { tokio::join!(server, client) };
        let (server, client) = tokio::time::timeout(self.timeout, both)
            .await
            .expect("the scenario should have ended long before this");

        Meeting {
            server: server.expect("the server side is preconnected"),
            client: client.expect("the client side is preconnected"),
        }
    }
}

/// One router, and a raw peer on the other end of the socket.
///
/// For the tests a [`Scenario`] cannot express, because the peer is not a connection: one that
/// sends bytes no packet accounts for, or one that never reads what it asked for.
pub struct Served {
    router: Router<Notes>,
    config: Options,
    buffer: usize,
}

impl Served {
    /// Prepares to run `router` against a raw peer.
    pub fn new(router: Router<Notes>) -> Self {
        Self {
            router,
            config: Options::default(),
            buffer: 4096,
        }
    }

    /// Adjusts the connection options.
    pub fn config(mut self, with: impl FnOnce(&mut Options)) -> Self {
        with(&mut self.config);
        self
    }

    /// How many bytes fit in the socket pair before a write blocks.
    pub fn buffer(mut self, bytes: usize) -> Self {
        self.buffer = bytes;
        self
    }

    /// Starts the connection and hands back the peer.
    pub fn start(self) -> (RawClient, JoinHandle<Outcome<Notes>>) {
        let (server_io, client_io) = tokio::io::duplex(self.buffer);
        let connection = client(server_io, self.router).config(self.config);
        (
            RawClient::new(client_io),
            tokio::spawn(async move { connection.connect().await.expect("preconnected") }),
        )
    }
}

/// How both sides of a [`Scenario`] ended.
pub struct Meeting {
    /// The answering side.
    pub server: Outcome<Notes>,
    /// The side that dialled and spoke first.
    pub client: Outcome<Notes>,
}

impl Meeting {
    /// Asserts that neither side failed, naming whichever did.
    pub fn expect_clean(&self) -> &Self {
        assert!(
            self.server.error.is_none(),
            "the server failed: {:?}",
            self.server.error,
        );
        assert!(
            self.client.error.is_none(),
            "the client failed: {:?}",
            self.client.error,
        );
        self
    }

    /// Why the server ended, or [`None`] if it closed cleanly.
    pub fn server_ending(&self) -> Option<&'static str> {
        self.server.error.as_ref().map(|error| error.reason())
    }

    /// Why the client ended, or [`None`] if it closed cleanly.
    pub fn client_ending(&self) -> Option<&'static str> {
        self.client.error.as_ref().map(|error| error.reason())
    }

    /// Whether the server's ending was the peer's doing, which is what decides the log level.
    ///
    /// Panics if the server closed cleanly, because there is nobody to blame for that.
    pub fn server_blamed_peer(&self) -> bool {
        self.server
            .error
            .as_ref()
            .expect("the server ended cleanly")
            .is_peer_error()
    }

    /// Whether the client's ending was the peer's doing.
    ///
    /// Panics if the client closed cleanly.
    pub fn client_blamed_peer(&self) -> bool {
        self.client
            .error
            .as_ref()
            .expect("the client ended cleanly")
            .is_peer_error()
    }

    /// What the server ended with, spelled out for a message.
    pub fn server_error(&self) -> String {
        self.server
            .error
            .as_ref()
            .map_or_else(|| "no error".to_owned(), ToString::to_string)
    }

    /// What the server recorded.
    pub fn server_saw(&self) -> Vec<String> {
        self.server.state.lines()
    }

    /// What the client recorded.
    pub fn client_saw(&self) -> Vec<String> {
        self.client.state.lines()
    }
}
