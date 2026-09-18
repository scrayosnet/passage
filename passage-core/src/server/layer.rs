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
