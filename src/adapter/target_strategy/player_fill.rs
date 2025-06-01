use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::PlayerFillTargetStrategy as PlayerFillConfig;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct PlayerFillTargetSelectorStrategy {
    field: String,
    max_players: u32,
}

impl PlayerFillTargetSelectorStrategy {
    pub fn new(config: PlayerFillConfig) -> Self {
        Self {
            field: config.field,
            max_players: config.max_players,
        }
    }
}

#[async_trait]
impl TargetSelectorStrategy for PlayerFillTargetSelectorStrategy {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error> {
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
            .map(|(target, _)| target.address);
        Ok(target)
    }
}
