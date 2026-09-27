//! A server that accepts what a test tells it to, when the test says so.

use super::notes::Notes;
use passage_core::router::Router;
use passage_core::server::{Listener, Server};
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// The peer a [`TestListener`] reports, carrying the notes every connection records into.
///
/// The notes travel with the address because that is the one thing a state factory is handed. It
/// keeps the factory a plain function with nothing captured, so the server's type can be named --
/// and a layer that rewrites the address is rewriting the same value the factory will see.
#[derive(Clone, Debug)]
pub struct Peer {
    /// Where the connection came from.
    pub addr: SocketAddr,
    /// Where every connection of this server writes what it saw.
    pub notes: Notes,
}

impl Peer {
    /// The port, which is how the tests tell one peer from another.
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Returns a copy reported from `port` instead.
    pub fn at(&self, port: u16) -> Self {
        let mut peer = self.clone();
        peer.addr.set_port(port);
        peer
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.addr)
    }
}

/// What the test hands to the accept loop.
pub type Accepted = io::Result<(DuplexStream, Peer)>;

/// A listener fed by a channel: the test decides what is accepted, and when.
///
/// Three lines of trait, which is the point: a listener is a *source* of sockets and nothing else,
/// and that one can be built out of a channel at all is part of what is being tested.
pub struct TestListener {
    incoming: mpsc::UnboundedReceiver<Accepted>,
}

impl Listener for TestListener {
    type Io = DuplexStream;
    type Addr = Peer;

    async fn accept(&mut self) -> Accepted {
        match self.incoming.recv().await {
            Some(accepted) => accepted,
            // Nothing more is coming. A quiet listener blocks; it does not report an error in a
            // loop, and the accept loop must not treat "quiet" as anything at all.
            None => std::future::pending().await,
        }
    }
}

/// The other end of a [`TestListener`].
pub struct Incoming {
    sender: mpsc::UnboundedSender<Accepted>,
    notes: Notes,
}

impl Incoming {
    /// Presents a connection from `port`, and returns the socket the peer holds.
    pub fn connect(&self, port: u16) -> DuplexStream {
        let (server_io, client_io) = tokio::io::duplex(4096);
        let peer = Peer {
            addr: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)),
            notes: self.notes.clone(),
        };
        self.sender
            .send(Ok((server_io, peer)))
            .expect("the accept loop is running");
        client_io
    }

    /// Presents a failed accept.
    pub fn fail(&self, kind: io::ErrorKind) {
        self.sender
            .send(Err(io::Error::new(kind, "as if the peer had gone away")))
            .expect("the accept loop is running");
    }
}

/// The state factory, as a function so that the server's type can be named.
fn notes_of(peer: &Peer) -> Notes {
    peer.notes.push(format!("accepted {}", peer.addr));
    peer.notes.clone()
}

/// A server built for a test: a channel for a listener, and one shared set of notes.
pub type TestServer = Server<TestListener, fn(&Peer) -> Notes, Arc<Router<Notes>>, ()>;

/// Everything a test needs to run a server and talk to it.
///
/// The server is handed over unspawned, so a test that wants a layer, a connection limit or a drain
/// timeout adds it and spawns the result itself:
///
/// ```ignore
/// let Harness { server, incoming, notes, shutdown } = Harness::new(router);
/// let task = tokio::spawn(server.layer(rate_limiter).serve());
/// ```
pub struct Harness {
    /// The server, ready to be finished and spawned.
    pub server: TestServer,
    /// Presents connections and accept failures.
    pub incoming: Incoming,
    /// What every connection of this server recorded.
    pub notes: Notes,
    /// Cancelling this stops the accept loop and every live connection.
    pub shutdown: CancellationToken,
}

impl Harness {
    /// Builds a server that routes with `router`.
    pub fn new(router: Router<Notes>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let notes = Notes::default();
        let shutdown = CancellationToken::new();

        Self {
            server: Server::new(TestListener { incoming: rx })
                .state(notes_of as fn(&Peer) -> Notes)
                .dispatch(Arc::new(router))
                .graceful_shutdown(shutdown.clone()),
            incoming: Incoming {
                sender: tx,
                notes: notes.clone(),
            },
            notes,
            shutdown,
        }
    }

    /// Spawns the server as it stands.
    pub fn start(self) -> Running {
        Running {
            incoming: self.incoming,
            notes: self.notes,
            shutdown: self.shutdown,
            task: tokio::spawn(self.server.serve()),
        }
    }
}

/// A running server, and the handles a test drives it with.
pub struct Running {
    /// Presents connections and accept failures.
    pub incoming: Incoming,
    /// What every connection of this server recorded.
    pub notes: Notes,
    /// Cancelling this stops the accept loop and every live connection.
    pub shutdown: CancellationToken,
    /// The accept loop itself.
    pub task: JoinHandle<()>,
}
