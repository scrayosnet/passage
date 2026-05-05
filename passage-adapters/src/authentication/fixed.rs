use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Client, Player, error::Result, metrics, reject_reason};
use tokio::time::Instant;
use tracing::trace;

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
        _client: &Client,
        _player: &Player,
        _shared_secret: &[u8],
        _encoded_public: &[u8],
    ) -> Result<Profile> {
        trace!("authenticating fixed profile");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        self.profile
            .clone()
            .ok_or_else(|| reject_reason(ADAPTER_TYPE, "no_profile"))
    }
}
