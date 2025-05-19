pub mod fixed;
pub mod grpc;
pub mod none;

use crate::adapter::status::Protocol;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::Error;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
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

async fn strategize(
    strategy: Arc<dyn TargetSelectorStrategy>,
    client_addr: &SocketAddr,
    server_addr: (&str, u16),
    protocol: Protocol,
    username: &str,
    user_id: &Uuid,
    targets: &[Target],
) -> Result<Option<SocketAddr>, Error> {
    let selected_target = strategy
        .select(
            client_addr,
            server_addr,
            protocol,
            username,
            user_id,
            targets,
        )
        .await?;

    Ok(selected_target)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    pub identifier: String,
    pub address: SocketAddr,
    #[serde(default)]
    pub meta: HashMap<String, String>,
}
