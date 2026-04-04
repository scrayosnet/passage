use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Protocol, Reason, ReasonExt, error::Result, metrics};
use std::net::SocketAddr;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "fixed_authentication_adapter";

#[derive(Debug, Default)]
pub struct FixedAuthenticationAdapter {
    profile: Option<Profile>,
}

impl FixedAuthenticationAdapter {
    pub fn new(profile: Option<Profile>) -> Self {
        Self { profile }
    }
}

impl AuthenticationAdapter for FixedAuthenticationAdapter {
    #[tracing::instrument(skip_all)]
    async fn authenticate(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        _shared_secret: &[u8],
        _encoded_public: &[u8],
    ) -> Result<Reason<Profile>> {
        trace!("authenticating fixed profile");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        Ok(self.profile.clone().reason(None))
    }
}
