use std::net::SocketAddr;
use uuid::Uuid;
use crate::protocol::Error;
use crate::status::Protocol;
use crate::target_selector::Target;
use crate::target_selector_strategy::TargetSelectorStrategy;

#[derive(Default)]
pub struct SimpleTargetSelectorStrategy {

}

impl TargetSelectorStrategy for SimpleTargetSelectorStrategy {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: &(String, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
        targets: &[Target]
    ) -> Result<Option<SocketAddr>, Error> {
        todo!()
    }
}
