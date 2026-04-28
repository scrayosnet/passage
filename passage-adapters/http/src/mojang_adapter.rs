use crate::HTTP_CLIENT;
use passage_adapters::authentication::{AuthenticationAdapter, Profile, minecraft_hash};
use passage_adapters::{Client, Player, metrics, reject};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "mojang_authentication_adapter";

#[derive(Default)]
pub struct MojangAdapter {
    server_id: String,
}

impl MojangAdapter {
    pub fn with_server_id(mut self, server_id: String) -> Self {
        self.server_id = server_id;
        self
    }

    async fn authenticate(
        &self,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        // calculate the minecraft hash for this secret, key and username
        let hash = minecraft_hash(&self.server_id, shared_secret, encoded_public);

        // issue a request to the Mojang authentication endpoint
        let username = &player.name;
        let url = format!(
            "https://sessionserver.mojang.com/session/minecraft/hasJoined?username={username}&serverId={hash}"
        );
        let response = HTTP_CLIENT
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
            })?;

        // if the response is empty, then the client did not make an auth request
        if response.status() == 204 {
            return Err(reject(ADAPTER_TYPE));
        }

        // parse the response profile
        let profile =
            response
                .json()
                .await
                .map_err(|err| passage_adapters::Error::FailedParse {
                    adapter_type: "mojang",
                    cause: Box::new(err),
                })?;
        Ok(profile)
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
        _client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        let start = Instant::now();
        let profile = self
            .authenticate(player, shared_secret, encoded_public)
            .await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        profile
    }
}
