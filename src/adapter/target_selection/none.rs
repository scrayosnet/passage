use crate::adapter::target_selection::TargetSelector;
use crate::protocol::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Default)]
pub struct NoneTargetSelector;

#[async_trait]
impl TargetSelector for NoneTargetSelector {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Option<SocketAddr>, Error> {
        Ok(None)
    }
}
