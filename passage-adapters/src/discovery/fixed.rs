use crate::discovery::DiscoveryAdapter;
use crate::{Target, error::Result, metrics};
use tokio::time::Instant;
use tracing::trace;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "fixed_discovery_adapter";

#[derive(Debug)]
pub struct FixedDiscoveryAdapter {
    targets: Vec<Target>,
}

impl FixedDiscoveryAdapter {
    pub fn new(targets: Vec<Target>) -> Self {
        Self { targets }
    }
}

impl DiscoveryAdapter for FixedDiscoveryAdapter {
    #[tracing::instrument(skip_all)]
    async fn discover(&self) -> Result<Vec<Target>> {
        trace!(len = self.targets.len(), "passing fixed targets");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        Ok(self.targets.clone())
    }
}
