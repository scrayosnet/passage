use crate::protocol::Error;
use crate::status::Protocol;
use crate::target_selector::TargetSelector;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub struct SimpleTargetSelector {
    target: Option<SocketAddr>,
}

impl SimpleTargetSelector {
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
impl TargetSelector for SimpleTargetSelector {
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
