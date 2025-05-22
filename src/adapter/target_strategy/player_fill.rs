use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct PlayerFillTargetSelectorStrategy {
    field: String,
    max_players: u32,
}

impl PlayerFillTargetSelectorStrategy {
    pub fn new(field: String, max_players: u32) -> Self {
        Self { field, max_players }
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
        Ok(None)
    }
}
