use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct FixedFilterAdapter {
    // TODO add filter configuration
}

impl FixedFilterAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

impl FilterAdapter for FixedFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn filter(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
        trace!(len = targets.len(), "filtering targets");
        Ok(targets)
    }
}
