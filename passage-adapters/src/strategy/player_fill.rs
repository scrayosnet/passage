use crate::strategy::StrategyAdapter;
use crate::{Protocol, Target, error::Result};
use std::net::SocketAddr;
use uuid::Uuid;

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
        Ok(target)
    }
}
