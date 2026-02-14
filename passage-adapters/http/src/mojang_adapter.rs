use crate::HTTP_CLIENT;
use passage_adapters::Protocol;
use passage_adapters::authentication::{AuthenticationAdapter, Profile, minecraft_hash};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Default)]
pub struct MojangAdapter {
    server_id: String,
}

impl MojangAdapter {
    pub fn with_server_id(mut self, server_id: String) -> Self {
        self.server_id = server_id;
        self
    }
}

impl Debug for MojangAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "MojangAdapter")
    }
}

impl AuthenticationAdapter for MojangAdapter {
    async fn authenticate(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        user: (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        // calculate the minecraft hash for this secret, key and username
        let hash = minecraft_hash(&self.server_id, shared_secret, encoded_public);

        // issue a request to Mojang's authentication endpoint
        let url = format!(
            "https://sessionserver.mojang.com/session/minecraft/hasJoined?username={}&serverId={}"
            user.0,
            hash,
        );
        let profile = HTTP_CLIENT
            .get(&url)
            .send()
            .await
            .map_err(|err| passage_adapters::Error::FailedFetch {
                adapter_type: "mojang",
                cause: Box::new(err),
            })?
            .error_for_status()
            .map_err(|err| passage_adapters::Error::FailedFetch {
                adapter_type: "mojang",
                cause: Box::new(err),
            })?
            .json()
            .await
            .map_err(|err| passage_adapters::Error::FailedParse {
                adapter_type: "mojang",
                cause: Box::new(err),
            })?;
        Ok(profile)
    }
}
