```rs
use std::ops::ControlFlow;

// TODO: https://claude.ai/chat/42383fe5-eea1-4055-a015-1c17fabe21a5

pub type ProtocolVersion = i32;
pub type VarInt = i32;
pub trait Phase: 'static { const ID: PhaseId; }

pub struct PacketFrame;

pub trait Packet {
    /// Gets the packet ID for the given protocol version. It the packet is not supported for that
    /// version, `None` is returned.
    fn id(version: ProtocolVersion) -> Option<VarInt>;

    /// Accepts a visitor for the packet. This allows packets to be passed by reference with temporary
    /// lifetime to a handler.
    fn accept<V: PacketVisitor<Self>>(&self, visitor: &mut V) -> Result<(), Error> {
        visitor.on(self)
    }
}

pub struct PacketA;
impl Packet for PacketA { fn id(version: ProtocolVersion) -> Option<VarInt> { None } }

pub struct PacketB;
impl Packet for PacketB { fn id(version: ProtocolVersion) -> Option<VarInt> { None } }

// TODO could be folded into the driver?
pub struct PacketRegistry;

/// A handler for packets using the visitor pattern.
pub trait PacketVisitor<T: Packet> {
    fn on(&mut self, packet: &'a T) -> Result<(), Error> { Ok(()) }
}

/// The type alias [`PacketVisitor`] for the [`PacketRegistry`]. It bundles all visitors supported by
/// the registry and is automatically implemented for all supporting handlers.
pub trait PacketRegistryVisitor = PacketVisitor<Packet>;
impl PacketRegistryVisitor for V where V: PacketVisitor<PacketA> + PacketVisitor<PacketB> {}

impl PacketRegistry {
    fn dispatch<'a, V: PacketRegistryVisitor>(visitor: &'a mut V, version: ProtocolVersion, frame: &'a PacketFrame) -> Result<(), Error> {
        // TODO: This should be defined at once place, not two (other is the packet)
        match (version, frame.id) {
            // Passing the version into the packet allows us to reduce the number of packet implementations (the packet will be generic over the version).
            (_, 0x04) => PacketA::decode(&frame.data, version)?.accept(visitor),
            (775, 0x05) => PacketB::decode(&frame.data, version)?.accept(visitor),
        }
    }
}

pub enum DriverOp {
    // Sends a packet to the remote.
    Send(Packet),
    // Calls the channel, allowing the caller to drain the operations (until this marker).
    Drain(oneshot::Sender<()>),
}

pub struct Ctx<S> {
    /// The per-connection state, managed by the handler.
    state: Arc<Mutex<S>>,
    phase: VarInt,
    protocol_version: ProtocolVersion,
    ops: mpsc::UnboundedSender<Op>,
}

pub trait Hooks<S> {
    // general
    fn on_tick(&mut self, ctx: &Ctx<S>) -> Flow<Error>;

    // handshake
    fn on_handshake_intention(&self, ctx: &DriverCtx, packet: ()) -> Flow<Error>;

    // status
    fn on_status_status_request(&self, packet: &()) -> Flow<Error>;
    fn on_status_status_response(&self, packet: &()) -> Flow<Error>;
    fn on_status_ping_request(&self, packet: &()) -> Flow<Error>;
    fn on_status_pong_response(&self, packet: &()) -> Flow<Error>;

    // configuration
    fn on_configuration_disconnect(&self, packet: &()) -> Flow<Error>;
    fn on_configuration_hello(&self, packet: &()) -> Flow<Error>;
    fn on_configuration_login_finished(&self, packet: &()) -> Flow<Error>;
}

struct Driver<H> {
    framed: Framed<TcpStream, PacketCodec>,
    conn:   Conn<S>,
    ops:    mpsc::UnboundedReceiver<Op>,
    sender:    mpsc::UnboundedSender<Op>,
    shutdown: CancellationToken,
    pending: JoinSet,
    hooks: H
}

impl Driver<H> where H: Hooks {
    pub fn new(stream: TcpStream, hooks: H) -> Self {
        Self { stream, hooks }
    }

    pub fn ctx(&mut self) -> Ctx<S> {
        TODO()
    }

    pub fn handel_ops(&mut self, ops: Option<Op>) -> Result<(), Error> {
        // TODO implement
        Ok(())
    }

    pub fn handle_error(&mut self, err: Error) {

    }

    pub fn handle_flow(&mut self, flow: Flow<ProtocolError>) -> Result<(), Error> {
        match flow {
            Flow::Ready(result) => result,
            Flow::Pending(future) => {
                self.pending.spawn(future);
                Ok(())
            },
        }
    }

    pub fn listen(&mut self) -> Result<(), Error> {
        // Create a new context for the iteration
        let ctx = self.ctx();
        loop {
            let ctx = ctx.clone();
            select! {
                biased;
                // First, any pending operations are handled.
                ops = self.ops.recv() => {
                    self.handle_ops(ops).await?;
                },

                // Then, any async tasks that sent these operations are joined. This frees memory.
                result = self.pending.join_next() => {
                    if let Err(err) = result {
                        self.handle_error(err);
                        break;
                    }
                },

                // Then, if the connection is shutdown, then the loop is exited.
                _ = self.shutdown.cancelled() => {
                    self.handle_error(Error::Timeout);
                    break;
                },

                // Then, the ticks are handled to ensure that keep alive packets are sent.
                tick = self.interval.tick() => {
                    let result = self.handle_flow(self.handle_tick(tick));
                    if let Err(err) = result {
                        self.handle_error(err);
                        break;
                    }
                },

                // Finally, the next packet is read from the stream.
                frame = self.stream.next() => {
                    // Stop all on connection close. There is nothing we can do about it.
                    let Some(frame) = frame else {
                        self.shutdown.cancel();
                        break;
                    };

                    // Dispatch the next frame
                    let result = self.handle_flow(self.dispatch(frame));
                    if let Err(err) = result {
                        self.handle_error(err);
                        break;
                    }
                },
            };
        };

        // Close the connection, notifying all tasks and waiting for them to finish.
        self.shutdown.cancel();
        self.pending.join_all().await;
    }

    fn dispatch(frame: &PacketFrame) -> Result<(), Error> {
        // TODO: The match should be defined at once place, not two (other is the packet)
        match (version, frame.id) {
            // Passing the version into the packet allows us to reduce the number of packet implementations (the packet will be generic over the version).
            (_, 0x04) => PacketA::decode(&frame.data, version)?.accept(visitor),
            (775, 0x05) => PacketB::decode(&frame.data, version)?.accept(visitor),
        }
    }
}

// TODO: Handler result type, allows sync handlers to prevent async statemachine for each packet -> only where required
#[must_use]
pub enum Flow<'a, E> {
    Ready(Result<(), E>),
    Pending(BoxFuture<'a, Result<(), E>>),
}
impl<'a, E> Flow<'a, E> {
    pub const OK: Self = Flow::Ready(Ok(()));

    pub fn later(f: impl Future<Output = Result<(), E>> + Send + 'a) -> Self {
        Flow::Pending(Box::pin(f))
    }
}
impl<'a, E> From<Result<(), E>> for Flow<'a, E> {
    fn from(r: Result<(), E>) -> Self { Flow::Ready(r) }
}

// TODO initialize codec with only 512B size (packets are small)
// TODO use IoSlice for sending packets (getting size): https://medium.com/@bhagyarana80/top-10-rust-concurrency-patterns-for-zero-copy-speed-60aac19e0d4a
```