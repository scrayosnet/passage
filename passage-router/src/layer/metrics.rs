//! The layer that counts connections, and the socket wrapper that times them.

use crate::metrics;
use passage_core::router::Layer;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

/// Counts every connection that makes it past the layers in front of it, and times it.
///
/// This belongs last in the stack: the layers before it are the ones that refuse, and a connection
/// they refused is not one that was open. What they refuse they count themselves, so that
/// `listener_requests` adds up to everything that was accepted from the socket.
pub struct MetricsLayer;

impl<Io, Addr> Layer<Io, Addr> for MetricsLayer
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Addr: Send + 'static,
{
    type Io = Tracked<Io>;

    async fn admit(&self, io: Io, addr: Addr) -> Option<(Self::Io, Addr)> {
        metrics::requests::accept();
        metrics::open_connections::inc();
        Some((Tracked::new(io), addr))
    }
}

/// A socket that reports its own end.
///
/// The connection is over when its socket is dropped, and that is the only point both a clean close
/// and a panicking handler pass through -- so it is where the gauge comes back down. Anything else
/// would leak the count on the paths that do not return normally.
pub struct Tracked<Io> {
    io: Io,
    started: Instant,
}

impl<Io> Tracked<Io> {
    fn new(io: Io) -> Self {
        Self {
            io,
            started: Instant::now(),
        }
    }
}

impl<Io> Drop for Tracked<Io> {
    fn drop(&mut self) {
        metrics::open_connections::dec();
        metrics::connection_duration::record(self.started);
    }
}

// `Io: Unpin` is guaranteed by the `Layer` bounds, so the projection below needs no pin machinery.
impl<Io: AsyncRead + Unpin> AsyncRead for Tracked<Io> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl<Io: AsyncWrite + Unpin> AsyncWrite for Tracked<Io> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    #[tokio::test]
    async fn a_tracked_socket_is_still_the_socket() {
        // The whole wrapper is only worth having if nothing downstream can tell it is there.
        let (a, mut b) = duplex(64);
        let mut tracked = Tracked::new(a);

        tracked.write_all(b"hello").await.expect("writes");
        tracked.flush().await.expect("flushes");

        let mut buf = [0_u8; 5];
        b.read_exact(&mut buf).await.expect("reads");
        assert_eq!(&buf, b"hello");

        b.write_all(b"world").await.expect("writes");
        let mut buf = [0_u8; 5];
        tracked.read_exact(&mut buf).await.expect("reads");
        assert_eq!(&buf, b"world");
    }

    #[tokio::test]
    async fn the_layer_admits_everything_it_is_given() {
        // It counts; it does not decide. A layer that refused here would refuse silently, because
        // there is nothing it could be refusing *for*.
        let admitted = MetricsLayer.admit(duplex(64).0, 25_565_u16).await;
        assert_eq!(admitted.expect("admitted").1, 25_565);
    }
}
