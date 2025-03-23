use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use crate::status_supplier::StatusSupplier;
use std::net::SocketAddr;

#[derive(Default)]
pub struct HiddenStatusSupplier;

impl StatusSupplier for HiddenStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: &(String, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(None)
    }
}
