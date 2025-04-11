use crate::adapter::target_selection::TargetSelector;
use crate::protocol::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct FixedTargetSelector {
    target: Option<SocketAddr>,
}

impl FixedTargetSelector {
    pub fn new() -> Self {
        Self { target: None }
    }

    pub fn from_target(target: impl Into<SocketAddr>) -> Self {
        Self {
            target: Some(target.into()),
        }
    }
}

#[async_trait]
impl TargetSelector for FixedTargetSelector {
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user_id: &Uuid,
        _username: &str,
    ) -> Result<Option<SocketAddr>, Error> {
        Ok(self.target)
    }
}
