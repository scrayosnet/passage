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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn a_socket_somebody_else_opened_is_handed_over_once() {
        // What a test uses: one half of a duplex pair is a connection that was never dialed, and a
        // client should not have to grow a second entry point to accept one.
        let (io, _peer) = duplex(64);
        let mut connector = Connected::new(io, "in-process");

        let (_io, addr) = connector.connect().await.expect("the socket is there");
        assert_eq!(addr, "in-process");

        let err = connector.connect().await.expect_err("there is only one");
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[tokio::test]
    async fn a_closure_is_a_connector_for_a_socket_the_crate_knows_nothing_about() {
        let mut connector = connect_with(|| async { Ok((duplex(64).0, "made up")) });
        let (_io, addr) = connector.connect().await.expect("connects");
        assert_eq!(addr, "made up");
        // Unlike `Connected`, a closure can be dialed again.
        assert!(connector.connect().await.is_ok());
    }

    #[tokio::test]
    async fn an_address_dials_itself_and_reports_where_it_landed() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let mut addr = listener.local_addr().expect("bound");

        let (_io, reported) = addr.connect().await.expect("connects");
        assert_eq!(reported, addr);
        let (_accepted, _peer) = listener.accept().await.expect("accepts");
    }

    #[tokio::test]
    async fn a_refused_dial_is_an_error_the_caller_can_read() {
        // Nothing listens on a port we bound and dropped.
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        let mut addr = listener.local_addr().expect("bound");
        drop(listener);

        let err = addr.connect().await.expect_err("nothing is listening");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
    }
}
