pub mod fixed;
pub mod none;

use crate::connection::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

#[async_trait]
pub trait TargetSelector: Send + Sync {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Option<SocketAddr>, Error>;
}

pub type TargetIdentifier = String;

#[derive(Debug, Clone)]
pub struct Target {
    pub identifier: TargetIdentifier,
    pub address: SocketAddr,
    pub meta: HashMap<String, String>,
}
