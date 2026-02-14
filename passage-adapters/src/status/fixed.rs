use crate::status::StatusAdapter;
use crate::{Protocol, ServerStatus, error::Result};
use std::net::SocketAddr;
use tracing::trace;

#[derive(Debug, Default)]
pub struct FixedStatusAdapter {
    status: Option<ServerStatus>,
    preferred_version: Protocol,
    min_version: Protocol,
    max_version: Protocol,
}

impl FixedStatusAdapter {
    pub fn new(
        status: Option<ServerStatus>,
        preferred_version: Protocol,
        min_version: Protocol,
        max_version: Protocol,
    ) -> Self {
        Self {
            status,
            preferred_version,
            min_version,
            max_version,
        }
    }
}

impl StatusAdapter for FixedStatusAdapter {
    #[tracing::instrument(skip_all)]
    async fn status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>> {
        trace!(has_status = self.status.is_some(), "passing fixed status");
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
