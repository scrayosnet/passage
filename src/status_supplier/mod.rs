pub mod none;
pub mod simple;

use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use std::net::SocketAddr;

pub trait StatusSupplier: Send {
    fn get_status(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
    ) -> impl Future<Output=Result<Option<ServerStatus>, Error>> + Send;
}
