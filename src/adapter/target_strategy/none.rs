use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::Error;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Default)]
pub struct NoneTargetSelectorStrategy;

#[async_trait]
impl TargetSelectorStrategy for NoneTargetSelectorStrategy {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
        _targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error> {
        Ok(None)
    }
}
