pub mod fixed;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod player_fill;

use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

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
    ) -> Result<Option<Target>, Error>;
}
