use crate::status::StatusAdapter;
use crate::{Client, Protocol, ServerStatus, error::Result, metrics};
use tokio::time::Instant;
use tracing::trace;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "fixed_status_adapter";

/// Status adapter that always returns the same pre-configured server status.
///
/// The protocol version reported in the status is adjusted dynamically: when the connecting
/// client's protocol version falls within `[min_version, max_version]`, the client's version is
/// echoed back so the client shows "compatible"; otherwise `preferred_version` is used.
///
/// Passing `None` for `status` makes the adapter return no status.
#[derive(Debug, Default)]
pub struct FixedStatusAdapter {
    status: Option<ServerStatus>,
    preferred_version: Protocol,
    min_version: Protocol,
    max_version: Protocol,
}

impl FixedStatusAdapter {
    /// Creates a new `FixedStatusAdapter`.
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
    async fn status(&self, client: &Client) -> Result<Option<ServerStatus>> {
        trace!(has_status = self.status.is_some(), "passing fixed status");
        let start = Instant::now();

        let stat = self.status.clone();
        let Some(mut stat) = stat else {
            metrics::adapter_duration::record(ADAPTER_TYPE, start);
            return Ok(None);
        };

        // set protocol version
        stat.version.protocol = self.preferred_version;
        if self.min_version <= client.protocol_version
            && client.protocol_version <= self.max_version
        {
            stat.version.protocol = client.protocol_version;
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(Some(stat))
    }
}
