use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use crate::status_supplier::StatusSupplier;
use std::net::SocketAddr;

#[derive(Default)]
pub struct SimpleStatusSupplier {
    status: Option<ServerStatus>,
}

impl SimpleStatusSupplier {
    pub fn from_status(status: impl Into<ServerStatus>) -> Self {
        Self {
            status: Some(status.into()),
        }
    }
}

impl StatusSupplier for SimpleStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: &(String, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        let stat = self.status.clone();
        let Some(mut stat) = stat else {
            return Ok(None);
        };
        stat.version.protocol = protocol;
        Ok(Some(stat))
    }
}
