use crate::strategy::StrategyAdapter;
use crate::{Protocol, Target, error::Result, metrics};
use std::net::SocketAddr;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "any_strategy_adapter";

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
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        Ok(targets.first().cloned())
    }
}
