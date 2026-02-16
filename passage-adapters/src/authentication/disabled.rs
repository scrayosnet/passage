use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Protocol, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

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
    ) -> Result<Profile> {
        trace!("skipping authentication");
        // TODO profile may need skin information, maybe provide default
        Ok(Profile {
            id: *user_id,
            name: user_name.to_string(),
            properties: vec![],
            profile_actions: vec![],
        })
    }
}
