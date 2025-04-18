pub mod none;
pub mod simple;

use crate::connection::Error;
use crate::status::{Protocol, ServerStatus};
use async_trait::async_trait;
use std::net::SocketAddr;

#[async_trait]
pub trait StatusSupplier: Send + Sync {
    async fn get_status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error>;
}
