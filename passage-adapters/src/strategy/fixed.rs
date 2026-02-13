use crate::strategy::StrategyAdapter;
use crate::{Protocol, Target, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct FixedStrategyAdapter {}

impl FixedStrategyAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StrategyAdapter for FixedStrategyAdapter {
    #[tracing::instrument(skip_all)]
    async fn select(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
        targets: Vec<Target>,
    ) -> Result<Option<Target>> {
        trace!(len = targets.len(), "selecting first target");
        Ok(targets.first().cloned())
    }
}
