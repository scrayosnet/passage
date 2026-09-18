use std::{fmt, io};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tracing::warn;

/// A [`Connector`] opens a connection socket.
pub trait Connector: Send + 'static {
    /// The underlying socket.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// The address type identifying the peer.
    type Addr: fmt::Debug + Send + 'static;

    /// Opens the connection. Returning an error must be safe to retry.
    fn connect(&mut self) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send;
}

impl Connector for std::net::SocketAddr {
    type Io = TcpStream;
    type Addr = std::net::SocketAddr;

    async fn connect(&mut self) -> io::Result<(TcpStream, std::net::SocketAddr)> {
        let io = TcpStream::connect(*self).await?;

        // Passage generally sends small packets. So we disable Nagle, which is the default for TCP.
        // This is only an optimization and not required.
        if let Err(err) = io.set_nodelay(true) {
            warn!(cause = %err, addr = ?self, "could not disable Nagle on a connected socket");
        }

        Ok((io, *self))
    }
}

/// A [`Connector`] built from a closure. See [`connect_with`].
pub struct ConnectorFn<F>(F);

impl<F, Fut, T, A> Connector for ConnectorFn<F>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = io::Result<(T, A)>> + Send,
    T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    A: fmt::Debug + Send + 'static,
{
    type Io = T;
    type Addr = A;

    fn connect(&mut self) -> impl Future<Output = io::Result<(T, A)>> + Send {
        self.0()
    }
}

/// Makes a [`Connector`] from a closure. Useful for sockets the crate knows nothing about.
pub fn connect_with<F, Fut, T, A>(connect: F) -> ConnectorFn<F>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = io::Result<(T, A)>> + Send,
{
    ConnectorFn(connect)
}

/// A [`Connector`] that hands over a socket somebody else already opened, once. A second call
/// reports [`io::ErrorKind::NotConnected`] because there is no second socket to give.
pub struct Connected<T, A>(Option<(T, A)>);

impl<T, A> Connected<T, A> {
    /// Wraps an open socket and the address to report for it.
    pub fn new(io: T, addr: A) -> Self {
        Self(Some((io, addr)))
    }
}

impl<T, A> Connector for Connected<T, A>
where
    T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    A: fmt::Debug + Send + 'static,
{
    type Io = T;
    type Addr = A;

    async fn connect(&mut self) -> io::Result<(T, A)> {
        self.0.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "the preconnected socket was already taken",
            )
        })
    }
}
