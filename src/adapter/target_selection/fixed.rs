use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{strategize, Target, TargetSelector};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::Error;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

pub struct FixedTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
    targets: Vec<Target>,
}

impl FixedTargetSelector {
    pub fn new(strategy: Arc<dyn TargetSelectorStrategy>, targets: impl Into<Vec<Target>>) -> Self {
        Self {
            strategy,
            targets: targets.into(),
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
    ) -> Result<Option<SocketAddr>, Error> {
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
