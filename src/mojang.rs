use crate::authentication::minecraft_hash;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use tracing::instrument;
use uuid::Uuid;

/// The shared http client (for mojang requests).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

/// Represents a single Minecraft user profile with all current properties.
///
/// Each Minecraft account is associated with exactly one profile that reflects the visual and
/// technical state that the player is in. Some fields can be influenced by the player while other
/// fields are strictly set by the system.
///
/// The `properties` usually only include one property called `textures`, but this may change over
/// time, so it is kept as an array as that is what's specified in the JSON. The `profile_actions`
/// are empty for non-sanctioned accounts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The unique identifier of the Minecraft user profile.
    pub id: Uuid,
    /// The current visual name of the Minecraft user profile.
    pub name: String,
    /// The currently assigned properties of the Minecraft user profile.
    #[serde(default)]
    pub properties: Vec<ProfileProperty>,
    /// The pending imposed moderative actions of the Minecraft user profile.
    #[serde(default)]
    pub profile_actions: Vec<String>,
}

/// Represents a single property of a Minecraft user profile.
///
/// A property defines one specific aspect of a user profile. The most prominent property is called
/// `textures` and contains information on the skin and visual appearance of the user. Each property
/// name is unique for an individual user.
///
/// All properties are cryptographic signed to verify the authenticity of the provided data. The
/// `signature` of the property is signed with Yggdrasil's private key and therefore its
/// authenticity can be verified by the Minecraft client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProfileProperty {
    /// The unique, identifiable name of the profile property.
    pub name: String,
    /// The base64 encoded value of the profile property.
    pub value: String,
    /// The base64 encoded signature of the profile property.
    /// Only provided if `?unsigned=false` is appended to url
    pub signature: Option<String>,
}

#[async_trait]
pub trait Mojang: Send + Sync {
    async fn authenticate(
        &self,
        username: &str,
        shared_secret: &[u8],
        server_id: &str,
        encoded_public: &[u8],
    ) -> Result<Profile, reqwest::Error>;
}

#[derive(Default)]
pub struct Api {}

#[async_trait]
impl Mojang for Api {
    #[instrument(skip_all)]
    async fn authenticate(
        &self,
        username: &str,
        shared_secret: &[u8],
        server_id: &str,
        encoded_public: &[u8],
    ) -> Result<Profile, reqwest::Error> {
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
