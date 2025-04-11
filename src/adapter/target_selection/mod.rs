pub mod fixed;

use crate::protocol::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::iter::Map;
use std::net::SocketAddr;
use uuid::Uuid;

#[async_trait]
pub trait TargetSelector: Send + Sync {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user_id: &Uuid,
        username: &str,
    ) -> Result<Option<SocketAddr>, Error>;
}

#[derive(Debug, Clone)]
pub struct Target {
    pub identifier: String,
    pub address: SocketAddr,
    pub meta: Map<String, String>,
}
