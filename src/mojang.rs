use crate::authentication::minecraft_hash;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use uuid::Uuid;

/// The shared http client (for mojang requests).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    /// The unique identifier of the Minecraft user profile.
    pub id: Uuid,
    /// The current visual name of the Minecraft user profile.
    pub name: String,
}

#[async_trait]
pub trait Mojang: Send + Sync {
    async fn authenticate(
        &self,
        username: &str,
        shared_secret: &[u8],
        server_id: &str,
        encoded_public: &[u8],
    ) -> Result<AuthResponse, reqwest::Error>;
}

#[derive(Default)]
pub struct Api {}

#[async_trait]
impl Mojang for Api {
    async fn authenticate(
        &self,
        username: &str,
        shared_secret: &[u8],
        server_id: &str,
        encoded_public: &[u8],
    ) -> Result<AuthResponse, reqwest::Error> {
        // calculate the minecraft hash for this secret, key and username
        let hash = minecraft_hash(server_id, shared_secret, encoded_public);

        // issue a request to Mojang's authentication endpoint
        let url = format!(
            "https://sessionserver.mojang.com/session/minecraft/hasJoined?username={username}&serverId={hash}"
        );
        let response = HTTP_CLIENT.get(&url).send().await?.error_for_status()?;

        // extract the fields of the response
        Ok(response.json().await?)
    }
}
