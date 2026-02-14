use crate::authentication::{AuthenticationAdapter, Profile};
use crate::{Protocol, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct FixedAuthenticationAdapter {
    profile: Profile,
}

impl FixedAuthenticationAdapter {
    pub fn new(profile: Profile) -> Self {
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
    ) -> Result<Profile> {
        trace!("authenticating fixed profile");
        Ok(self.profile.clone())
    }
}
