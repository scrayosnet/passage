use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::FixedTargetDiscovery as FixedConfig;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

pub struct FixedTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
    targets: Vec<Target>,
}

impl FixedTargetSelector {
    pub fn new(strategy: Arc<dyn TargetSelectorStrategy>, config: FixedConfig) -> Self {
        Self {
            strategy,
            targets: config.targets,
        }
    }

    pub fn new_empty(strategy: Arc<dyn TargetSelectorStrategy>) -> Self {
        Self {
            strategy,
            targets: vec![],
        }
    }
}

#[async_trait]
impl TargetSelector for FixedTargetSelector {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Option<Target>, Error> {
        strategize(
            Arc::clone(&self.strategy),
            client_addr,
            server_addr,
            protocol,
            username,
            user_id,
            &self.targets,
        )
        .await
    }
}
