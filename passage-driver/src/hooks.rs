use std::sync::Arc;
use tokio::sync::{oneshot, Mutex, mpsc};
use crate::error::DriverError;
use crate::flow::Flow;

// TODO move to other file/crate
pub type ProtocolVersion = i32;

// TODO move to other file/crate
#[derive(Clone, Debug)]
pub enum Phase {
    Handshake,
    Status,
    Configuration,
    Play,
}

#[non_exhaustive]
pub enum Op {
    // Sends a packet to the remote.
    Send(()),

    // Calls the channel, allowing the caller to drain the operations (until this marker).
    Drain(oneshot::Sender<()>),
}

/// The [`Ctx`] is passed to all hooks. It contains the per-connection state and the channel to send
/// [`Op`] to the driver. The context may be cloned. However, the phase state may become out-of-date
/// if the context is handled in an asynchronous scope (i.e., [`Flow::Pending`]).
#[derive(Clone, Debug)]
pub struct Ctx<S> {
    /// The per-connection state, managed by the handler.
    state: Arc<Mutex<S>>,

    /// The connection phase at the time of calling the hook. A hook may have updated this after that
    /// only if this context is handled in an asynchronous scope (i.e., [`Flow::Pending`]).
    phase: Phase,

    /// The protocol version of the connection. This value is fixed for the duration of the connection.
    protocol_version: ProtocolVersion,

    /// The channel to send operations asynchronously to the driver. The driver will prioritize
    /// clearing the operations backlock before anything else.
    ops: mpsc::UnboundedSender<Op>,
}

impl <S> Ctx<S> {
    /// Sends a packet to the driver. It only returns an error if the connection is closed.
    pub fn send(&self, value: ()) -> Result<(), DriverError> {
        self.ops.send(Op::Send(value)).map_err(|_| DriverError::Timeout)?;
        Ok(())
    }

    /// Drains the diver operations until this marker. It only returns an error if the connection is
    /// closed.
    pub async fn drain(&self) -> Result<(), DriverError> {
        let (tx, rx) = oneshot::channel();
        self.ops.send(Op::Drain(tx))?;
        // Wait for the drain operation to complete. This is only dropped (error) if the connection
        // is closed. So a timeout error is ok.
        rx.await.map_err(|_| DriverError::Timeout)?;
        Ok(())
    }
}

/// The [`Hooks`] are passed to the [`Driver`] to handle all protocol logic. The diver only handles
/// the packet parsing (based on the phase and protocol version) and connection lifecycle, while the
/// hooks handle the phase change and packet handling.
pub trait Hooks<S> {
    // general
    fn on_tick(&mut self, ctx: &Ctx<S>) -> Flow<'_, Result<(), DriverError>>;

    // handshake
    fn on_handshake_intention(&self, ctx: &Ctx<S>, packet: ()) -> Flow<'_, Result<(), DriverError>>;

    // status
    fn on_status_status_request(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_status_status_response(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_status_ping_request(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_status_pong_response(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;

    // configuration
    fn on_configuration_disconnect(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_configuration_hello(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_configuration_login_finished(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
}
