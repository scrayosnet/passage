use crate::adapter::target_selection::Target;
use crate::protocol::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

pub mod any;

#[async_trait]
pub trait TargetSelectorStrategy: Send {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<String>, Error>;
}
