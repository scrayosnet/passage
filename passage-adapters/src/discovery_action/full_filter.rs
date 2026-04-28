use crate::{Client, DiscoveryActionAdapter, Player, Target, error::Result, metrics};
use tokio::time::Instant;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "full_filter_adapter";

#[derive(Debug, Default)]
pub struct FullFilterAdapter {
    field: String,
    max_players: u32,
}

impl FullFilterAdapter {
    pub fn new(field: String, max_players: u32) -> Self {
        Self { field, max_players }
    }
}

impl DiscoveryActionAdapter for FullFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn apply(
        &self,
        _client: &Client,
        _player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        let start = Instant::now();
        targets.retain(|target| {
            let players = target
                .meta
                .get(&self.field)
                .and_then(|players| players.parse::<u32>().ok())
                .unwrap_or(0);
            players < self.max_players
        });
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(())
    }
}
