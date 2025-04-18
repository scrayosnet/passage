use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::connection::Error;
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
        let selected_target = self
            .strategy
            .select(
                client_addr,
                server_addr,
                protocol,
                username,
                user_id,
                &self.targets,
            )
            .await?;
        let address = selected_target
            .and_then(|identifier| self.targets.iter().find(|t| t.identifier == identifier));

        Ok(address.map(|target| target.address))
    }
}
