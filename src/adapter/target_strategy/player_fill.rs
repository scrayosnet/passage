use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::{TargetFilterExt, TargetSelectorStrategy};
use crate::config::{PlayerFillTargetStrategy as PlayerFillConfig, TargetFilter};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct PlayerFillTargetSelectorStrategy {
    field: String,
    max_players: u32,
    target_filter: HashMap<String, TargetFilter>,
}

impl PlayerFillTargetSelectorStrategy {
    #[must_use]
    pub fn new(config: PlayerFillConfig) -> Self {
        Self {
            field: config.field,
            max_players: config.max_players,
            // store as hashmap to improve performance
            target_filter: HashMap::from_iter(
                config
                    .target_filters
                    .into_iter()
                    .map(|filter| (filter.server_host.clone(), filter)),
            ),
        }
    }
}

#[async_trait]
impl TargetSelectorStrategy for PlayerFillTargetSelectorStrategy {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        (server_host, _): (&str, u16),
        _protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<Target>, Error> {
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
            .filter(|(target, _)| {
                let Some(filter) = self.target_filter.get(server_host) else {
                    return self.target_filter.is_empty();
                };
                filter.matches(target, username, user_id)
            })
            .max_by_key(|(_, players)| *players)
            .map(|(target, _)| target.clone());
        Ok(target)
    }
}
