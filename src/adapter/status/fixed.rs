use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, ServerVersion, StatusSupplier};
use crate::config::FixedStatus as FixedConfig;
use crate::config::ProtocolRange;
use async_trait::async_trait;
use serde_json::value::RawValue;
use std::net::SocketAddr;

#[derive(Default)]
pub struct FixedStatusSupplier {
    protocol: ProtocolRange,
    status: Option<ServerStatus>,
}

impl FixedStatusSupplier {
    pub fn new(config: FixedConfig, protocol: ProtocolRange) -> Self {
        let description = config
            .description
            .and_then(|str| RawValue::from_string(str).ok());
        Self {
            protocol,
            status: Some(ServerStatus {
                version: ServerVersion {
                    name: config.name,
                    protocol: 0,
                },
                players: None,
                description,
                favicon: config.favicon,
                enforces_secure_chat: config.enforces_secure_chat,
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
