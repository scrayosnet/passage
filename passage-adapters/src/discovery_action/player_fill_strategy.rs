use crate::{Client, DiscoveryActionAdapter, Player, Target, error::Result, metrics};
use std::cmp::Ordering::Equal;
use tokio::time::Instant;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "player_fill_strategy_adapter";

#[derive(Debug, Default)]
pub struct PlayerFillStrategyAdapter {
    field: String,
    max_players: u32,
}

impl PlayerFillStrategyAdapter {
    pub fn new(field: String, max_players: u32) -> Self {
        Self { field, max_players }
    }

    fn get_players(&self, target: &Target) -> Option<u32> {
        target
            .meta
            .get(&self.field)
            .and_then(|players| players.parse::<u32>().ok())
    }

    fn cmp(&self, a: &Target, b: &Target) -> std::cmp::Ordering {
        // Compare by priority.
        let order = a.priority.cmp(&b.priority);
        if order != Equal {
            return order;
        }

        // Compare by player count, get the target with the most players first.
        let players_a = self.get_players(a).unwrap_or(0);
        let players_b = self.get_players(b).unwrap_or(0);
        players_a.cmp(&players_b).reverse()
    }
}

impl DiscoveryActionAdapter for PlayerFillStrategyAdapter {
    #[tracing::instrument(skip_all)]
    async fn apply(
        &self,
        _client: &Client,
        _player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        let start = Instant::now();

        // Ensure that there are at least two targets. Otherwise, the following loop will panic.
        if targets.len() <= 1 {
            return Ok(());
        }

        // First, order the targets by priority and then by this comparator. After that, update the
        // priorities. The priorities are fully recomputed. If there are too many targets, then the
        // rest gets the max priority.
        targets.sort_by(|a, b| self.cmp(a, b));
        let mut priority = 0u16;
        for i in 0..targets.len() {
            targets[i].priority = priority;
            if i < targets.len() - 1 && self.cmp(&targets[i], &targets[i + 1]) != Equal {
                priority = priority.saturating_add(1);
            }
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(())
    }
}
