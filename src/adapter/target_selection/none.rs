use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

pub struct NoneTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
}

impl NoneTargetSelector {
    pub fn new(strategy: Arc<dyn TargetSelectorStrategy>) -> Self {
        Self { strategy }
    }
}

#[async_trait]
impl TargetSelector for NoneTargetSelector {
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
            &[],
        )
        .await
    }
}
