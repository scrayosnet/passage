use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::protocol::Error;
use crate::status::Protocol;
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
impl TargetSelectorStrategy for SimpleTargetSelector {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<String>, Error> {
        Ok(targets.first().map(|target| target.identifier.clone()))
    }
}
