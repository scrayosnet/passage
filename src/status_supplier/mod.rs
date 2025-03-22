pub mod simple;

use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use std::net::SocketAddr;

#[trait_variant::make(StatusSupplier: Send)]
pub trait LocalStatusSupplier {
    async fn get_status(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error>;
}
