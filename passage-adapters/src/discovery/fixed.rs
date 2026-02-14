use crate::discovery::DiscoveryAdapter;
use crate::{Target, error::Result};
use tracing::trace;

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
        Ok(self.targets.clone())
    }
}
