use thiserror::Error;
use tokio::sync::mpsc;
use crate::hooks::Op;

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("The connection was closed by the server.")]
    Timeout,

    #[error("Failed to send operation to driver: {0}")]
    SendOp(#[from] mpsc::error::SendError<Op>),
}
