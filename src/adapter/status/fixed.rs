use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, ServerVersion, StatusSupplier};
use crate::config::FixedStatus as FixedConfig;
use async_trait::async_trait;
use serde_json::value::RawValue;
use std::net::SocketAddr;

#[derive(Default)]
pub struct FixedStatusSupplier {
    status: Option<ServerStatus>,
    preferred_version: Protocol,
    min_version: Protocol,
    max_version: Protocol,
}

impl FixedStatusSupplier {
    #[must_use]
    pub fn new(config: FixedConfig) -> Self {
        let description = config
            .description
            .and_then(|str| RawValue::from_string(str).ok());
        Self {
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
            preferred_version: config.preferred_version,
            min_version: config.min_version,
            max_version: config.max_version,
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
        stat.version.protocol = self.preferred_version;
        if self.min_version <= protocol && protocol <= self.max_version {
            stat.version.protocol = protocol;
        }

        Ok(Some(stat))
    }
}
