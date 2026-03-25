use crate::strategy::StrategyAdapter;
use crate::{Protocol, Target, error::Result, metrics};
use std::net::SocketAddr;
use tokio::time::Instant;
use uuid::Uuid;

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
}

impl StrategyAdapter for PlayerFillStrategyAdapter {
    #[tracing::instrument(skip_all)]
    async fn strategize(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Option<Target>> {
        let start = Instant::now();
        let target = targets
            .iter()
            .map(|target| {
                // handle invalid metadata as max players
                let players = target
                    .meta
                    .get(&self.field)
                    .and_then(|players| players.parse::<u32>().ok())
                    .unwrap_or(0);
                (target, players)
            })
            .filter(|(_, players)| *players < self.max_players)
            .max_by_key(|(_, players)| *players)
            .map(|(target, _)| target.clone());
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(target)
    }
}
