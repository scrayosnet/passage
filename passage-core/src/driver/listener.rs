use std::{fmt, io};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tracing::warn;

/// A [`Listener`] accepts new connection sockets (e.g., TCP streams).
pub trait Listener: Send + 'static {
    /// The underlying socket.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// The address type identifying the peer.
    type Addr: fmt::Debug + Send + 'static;

    /// Accepts the next connection. Returning an error must be safe to retry.
    fn accept(&mut self) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send;
}

impl Listener for TcpListener {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        let (io, addr) = TcpListener::accept(self).await?;

        // Passage generally sends small packets. So we disable Nagle, which is the default for TCP.
        // This is only an optimization and not required.
        if let Err(err) = io.set_nodelay(true) {
            warn!(cause = %err, ?addr, "could not disable Nagle on an accepted socket");
        }

        Ok((io, addr))
    }
}
