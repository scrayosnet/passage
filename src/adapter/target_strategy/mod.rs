use crate::adapter::target_selection::{Target, TargetIdentifier};
use crate::connection::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod any;
pub mod none;

#[async_trait]
pub trait TargetSelectorStrategy: Send + Sync {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<TargetIdentifier>, Error>;
}
