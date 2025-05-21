use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{strategize, TargetSelector};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::Error;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

pub struct AgonesTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
    namespace: String,
}

impl AgonesTargetSelector {
    pub async fn new(
        strategy: Arc<dyn TargetSelectorStrategy>,
        namespace: String,
    ) -> Result<Self, Error> {
        Ok(Self {
            strategy,
            namespace,
        })
    }
}

#[async_trait]
impl TargetSelector for AgonesTargetSelector {
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
