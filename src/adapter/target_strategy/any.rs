use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::connection::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct AnyTargetSelectorStrategy;

#[async_trait]
impl TargetSelectorStrategy for AnyTargetSelectorStrategy {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<String>, Error> {
        Ok(targets.first().map(|target| target.identifier.clone()))
    }
}
