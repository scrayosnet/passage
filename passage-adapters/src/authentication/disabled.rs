use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Protocol, error::Result, metrics};
use std::net::SocketAddr;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "disabled_authentication_adapter";

#[derive(Debug, Default)]
pub struct DisabledAuthenticationAdapter {}

impl DisabledAuthenticationAdapter {
    pub fn new() -> Self {
        Self {}
    }
}

impl AuthenticationAdapter for DisabledAuthenticationAdapter {
    #[tracing::instrument(skip_all)]
    async fn authenticate(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        (user_name, user_id): (&str, &Uuid),
        _shared_secret: &[u8],
        _encoded_public: &[u8],
    ) -> Result<Option<Profile>> {
        trace!("skipping authentication");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        // TODO profile may need skin information, maybe provide default
        Ok(Some(Profile {
            id: *user_id,
            name: user_name.to_string(),
            properties: vec![],
            profile_actions: vec![],
        }))
    }
}
