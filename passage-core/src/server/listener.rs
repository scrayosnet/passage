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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    #[tokio::test]
    async fn a_tcp_listener_is_a_listener() {
        // The trait is three lines because a listener is a *source* of sockets and nothing else.
        let mut listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let bound = listener.local_addr().expect("bound");

        let dialling = tokio::spawn(async move { TcpStream::connect(bound).await });
        let (io, addr) = Listener::accept(&mut listener).await.expect("accepts");
        let dialled = dialling.await.expect("no panic").expect("connects");

        assert_eq!(addr, dialled.local_addr().expect("connected"));
        assert!(io.nodelay().expect("a live socket"), "Nagle is off");
    }

    #[tokio::test]
    async fn a_listener_can_be_accepted_from_more_than_once() {
        // Returning is not the end of it: the accept loop calls this until it is cancelled.
        let mut listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let bound = listener.local_addr().expect("bound");

        for _ in 0..2 {
            let dialling = tokio::spawn(async move { TcpStream::connect(bound).await });
            Listener::accept(&mut listener).await.expect("accepts");
            dialling.await.expect("no panic").expect("connects");
        }
    }
}
