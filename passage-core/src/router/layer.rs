//! Everything that happens to a socket before the protocol takes over.
//!
//! A PROXY header, a TLS handshake, a rate limiter: all the same shape, and the crate ships none of
//! them -- a layer belongs next to the thing it implements. Both halves take the same stack, so a
//! layer written for a [`Server`](crate::server::Server) works on a [`Client`](crate::client::Client).

use tokio::io::{AsyncRead, AsyncWrite};

/// A [`Layer`] intercepts incoming connections before the Minecraft protocol takes over. It should
/// be used to implement functions such as rate limiting, logging, and reverse proxy handling.
pub trait Layer<Io, Addr>: Send + Sync + 'static {
    /// The underlying socket.
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;

    /// Intercepts the connection. Return `None` to reject the connection.
    fn admit(&self, io: Io, addr: Addr) -> impl Future<Output = Option<(Self::Io, Addr)>> + Send;
}

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

/// Stacks layers together into a combined layer. The layers are called recursively. This mechanism
/// is used to configure multiple layers in order.
pub struct Stack<A, B>(A, B);

impl<A, B> Stack<A, B> {
    /// Stacks `b` behind `a`: `a` sees the socket first, `b` sees what `a` produced.
    pub fn new(a: A, b: B) -> Self {
        Self(a, b)
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{BufReader, DuplexStream, duplex};

    /// A layer in the shape of a PROXY header reader: it rewrites the address everything downstream
    /// sees, and refuses a peer it will not vouch for.
    struct RewritePort {
        refuse: u16,
    }

    impl<Io> Layer<Io, u16> for RewritePort
    where
        Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        type Io = Io;

        async fn admit(&self, io: Io, port: u16) -> Option<(Io, u16)> {
            // A real one would count this, and say whether it was a malformed header or an
            // untrusted source. Nothing downstream is told, and nothing downstream needs to be.
            (port != self.refuse).then_some((io, port + 1_000))
        }
    }

    /// A layer that changes the socket *type*, which is what a TLS layer does and the reason
    /// [`Layer::Io`] exists.
    struct Upgrade;

    impl<Io> Layer<Io, u16> for Upgrade
    where
        Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        type Io = BufReader<Io>;

        async fn admit(&self, io: Io, port: u16) -> Option<(BufReader<Io>, u16)> {
            Some((BufReader::new(io), port))
        }
    }

    fn socket() -> DuplexStream {
        duplex(64).0
    }

    #[tokio::test]
    async fn no_layer_is_a_layer_that_changes_nothing() {
        let admitted = ().admit(socket(), 25_565_u16).await;
        assert_eq!(admitted.expect("admitted").1, 25_565);
    }

    #[tokio::test]
    async fn a_closure_is_an_admission_check_with_nothing_to_count() {
        let layer = |port: &u16| *port != 2;
        assert!(layer.admit(socket(), 1_u16).await.is_some());
        assert!(layer.admit(socket(), 2_u16).await.is_none());
    }

    #[tokio::test]
    async fn layers_run_in_the_order_they_are_stacked() {
        // Only one order can produce this: the rewrite turns port 2 into 1002, and the closure
        // after it refuses 1002. Written the other way round the closure would see port 2, admit
        // it, and the connection would run.
        let stack = Stack::new(RewritePort { refuse: 0 }, |port: &u16| *port != 1_002);
        assert!(stack.admit(socket(), 2_u16).await.is_none());

        let admitted = stack.admit(socket(), 3_u16).await.expect("admitted");
        assert_eq!(
            admitted.1, 1_003,
            "the address downstream sees is the last one"
        );
    }

    #[tokio::test]
    async fn a_layer_that_refuses_stops_the_stack() {
        // Nothing behind a refusal runs, which is what makes a rate limiter cheap: the work it
        // saves is everything after it.
        let stack = Stack::new(RewritePort { refuse: 2 }, Upgrade);
        assert!(stack.admit(socket(), 2_u16).await.is_none());
        assert!(stack.admit(socket(), 1_u16).await.is_some());
    }

    #[tokio::test]
    async fn a_layer_can_change_the_socket_type_downstream() {
        // The connection ends up running on something the listener never produced.
        let stack = Stack::new(Upgrade, RewritePort { refuse: 0 });
        let (io, port) = stack.admit(socket(), 1_u16).await.expect("admitted");
        let _: BufReader<DuplexStream> = io;
        assert_eq!(port, 1_001);
    }
}
