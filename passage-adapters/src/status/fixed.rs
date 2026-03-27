use crate::status::StatusAdapter;
use crate::{Protocol, ServerStatus, error::Result, metrics};
use std::net::SocketAddr;
use tokio::time::Instant;
use tracing::trace;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "fixed_status_adapter";

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
        let start = Instant::now();

        let stat = self.status.clone();
        let Some(mut stat) = stat else {
            metrics::adapter_duration::record(ADAPTER_TYPE, start);
            return Ok(None);
        };

        // set protocol version
        stat.version.protocol = self.preferred_version;
        if self.min_version <= protocol && protocol <= self.max_version {
            stat.version.protocol = protocol;
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(Some(stat))
    }
}
