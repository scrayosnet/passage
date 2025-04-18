use crate::adapter::status::StatusSupplier;
use crate::connection::Error;
use crate::status::{Protocol, ServerStatus, ServerVersion};
use async_trait::async_trait;
use serde_json::value::RawValue;
use std::net::SocketAddr;

#[derive(Default)]
pub struct FixedStatusSupplier {
    protocol: crate::config::Protocol,
    status: Option<ServerStatus>,
}

impl FixedStatusSupplier {
    pub fn new(protocol: crate::config::Protocol, status: crate::config::FixedStatus) -> Self {
        let description = status
            .description
            .and_then(|str| RawValue::from_string(str).ok());
        Self {
            protocol,
            status: Some(ServerStatus {
                version: ServerVersion {
                    name: status.name,
                    protocol: 0,
                },
                players: None,
                description,
                favicon: status.favicon,
                enforces_secure_chat: status.enforces_secure_chat,
            }),
        }
    }
}

#[async_trait]
impl StatusSupplier for FixedStatusSupplier {
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
