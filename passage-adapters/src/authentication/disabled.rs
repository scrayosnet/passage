use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Client, Player, error::Result, metrics};
use tokio::time::Instant;
use tracing::trace;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "disabled_authentication_adapter";

/// Authentication adapter that skips all verification and accepts every player.
///
/// The returned profile mirrors the name and ID the client supplied during login. Use this adapter
/// only in environments where Mojang authentication is unavailable or undesired (e.g. offline-mode
/// test setups).
#[derive(Debug, Default)]
pub struct DisabledAuthenticationAdapter {}

impl DisabledAuthenticationAdapter {
    /// Creates a new `DisabledAuthenticationAdapter`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl AuthenticationAdapter for DisabledAuthenticationAdapter {
    #[tracing::instrument(skip_all)]
    async fn authenticate(
        &self,
        _client: &Client,
        player: &Player,
        _shared_secret: &[u8],
        _encoded_public: &[u8],
    ) -> Result<Profile> {
        trace!("skipping authentication");
        metrics::adapter_duration::record(ADAPTER_TYPE, Instant::now());
        // TODO profile may need skin information, maybe provide default
        Ok(Profile {
            id: player.id,
            name: player.name.clone(),
            properties: vec![],
            profile_actions: vec![],
        })
    }
}
