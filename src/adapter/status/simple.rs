use crate::adapter::status::StatusSupplier;
use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use async_trait::async_trait;
use std::net::SocketAddr;

#[derive(Default)]
pub struct SimpleStatusSupplier {
    protocol: crate::config::Protocol,
    status: Option<ServerStatus>,
}

impl SimpleStatusSupplier {
    pub fn from_status(protocol: crate::config::Protocol, status: impl Into<ServerStatus>) -> Self {
        Self {
            protocol,
            status: Some(status.into()),
        }
    }
}

#[async_trait]
impl StatusSupplier for SimpleStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        let stat = self.status.clone();
        let Some(mut stat) = stat else {
            return Ok(None);
        };

        // set protocol version
        stat.version.protocol = self.protocol.preferred;
        if self.protocol.min <= protocol && protocol <= self.protocol.max {
            stat.version.protocol = protocol;
        }

        Ok(Some(stat))
    }
}
