use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use async_trait::async_trait;
use std::net::SocketAddr;

#[derive(Default)]
pub struct NoneStatusSupplier;

#[async_trait]
impl StatusSupplier for NoneStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(None)
    }
}
