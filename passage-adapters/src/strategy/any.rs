use crate::strategy::StrategyAdapter;
use crate::{Protocol, Target, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct AnyStrategyAdapter {}

impl AnyStrategyAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StrategyAdapter for AnyStrategyAdapter {
    #[tracing::instrument(skip_all)]
    async fn strategize(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Option<Target>> {
        trace!(len = targets.len(), "selecting any target");
        Ok(targets.first().cloned())
    }
}
