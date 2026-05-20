use crate::discovery::DiscoveryAdapter;
use crate::{Client, Target, error::Result, metrics};
use tokio::time::Instant;
use tracing::trace;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "fixed_discovery_adapter";

/// Discovery adapter that always returns the same pre-configured list of targets.
///
/// Suitable for static environments or testing where backend server addresses are known in advance.
#[derive(Debug)]
pub struct FixedDiscoveryAdapter {
    /// The targets that should be returned for every request.
    targets: Vec<Target>,
}

impl FixedDiscoveryAdapter {
    /// Creates a new `FixedDiscoveryAdapter` with the given list of targets.
    pub fn new(targets: Vec<Target>) -> Self {
        Self { targets }
    }
}

impl DiscoveryAdapter for FixedDiscoveryAdapter {
    #[tracing::instrument(skip_all)]
    async fn discover(&self, _client: &Client) -> Result<Vec<Target>> {
        trace!(len = self.targets.len(), "passing fixed targets");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        Ok(self.targets.clone())
    }
}
