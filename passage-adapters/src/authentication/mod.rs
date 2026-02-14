pub mod fixed;

use crate::{Protocol, error::Result};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fmt::Debug;
use std::net::SocketAddr;
use uuid::Uuid;

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

pub trait AuthenticationAdapter: Debug + Send + Sync {
    fn authenticate(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> impl Future<Output = Result<Profile>> + Send;
}

/// Creates hash for the Minecraft protocol.
pub fn minecraft_hash(server_id: &str, shared_secret: &[u8], encoded_public: &[u8]) -> String {
    // create a new hasher instance
    let mut hasher = Sha1::new();

    // server id
    hasher.update(server_id);
    // shared secret
    hasher.update(shared_secret);
    // encoded public key
    hasher.update(encoded_public);

    // take the digest and convert it to Minecraft's format
    BigInt::from_signed_bytes_be(&hasher.finalize()).to_str_radix(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_hash() {
        let shared_secret = b"verysecuresecret";
        let encoded = b"verysecuresecret";
        let _ = minecraft_hash("justchunks", shared_secret, encoded);
    }
}
