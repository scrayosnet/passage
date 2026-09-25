pub mod disabled;
pub mod fixed;

use crate::{Client, Player, error::Result};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::fmt::Debug;
use uuid::Uuid;

/// The [`AuthenticationAdapter`] is used to provide custom logic for validating a connecting player
/// against an authentication authority. The default configuration intents using the HTTP adapter
/// `MojangAdapter,` which implements the official Minecraft authentication protocol.
///
/// Implementations verify that the player who sent the login request is who they claim to be.
/// The shared secret and encoded public key come from the Minecraft encryption handshake and are
/// forwarded verbatim to the backend for verification.
///
/// A successful call returns the player's full [`Profile`]. Returning [`Err`] causes the
/// connection to be dropped with an appropriate disconnect message.
pub trait AuthenticationAdapter: Debug + Send + Sync {
    /// Authenticates a connecting player.
    fn authenticate(
        &self,
        client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> impl Future<Output = Result<Profile>> + Send;
}

/// Represents a single Minecraft user profile with all current properties.
///
/// Each Minecraft account is associated with exactly one profile that reflects the visual and
/// technical state that the player is in. Some fields can be influenced by the player while other
/// fields are strictly set by the system.
///
/// The `properties` usually only include one property called `textures`, but this may change over
/// time, so it is kept as an array as that is what's specified in the JSON. The `profile_actions`
/// are empty for non-sanctioned accounts.
pub type Profile = passage_core::common::Profile;

/// Represents a single property of a Minecraft user profile.
///
/// A property defines one specific aspect of a user profile. The most prominent property is called
/// `textures` and contains information on the skin and visual appearance of the user. Each property
/// name is unique for an individual user.
///
/// All properties are cryptographic signed to verify the authenticity of the provided data. The
/// `signature` of the property is signed with Yggdrasil's private key and therefore its
/// authenticity can be verified by the Minecraft client.
pub type ProfileProperty = passage_core::common::ProfileProperty;

/// Computes the Minecraft session server hash from the server ID, shared secret, and encoded
/// public key.
///
/// The resulting string uses Minecraft's non-standard signed hex format: negative values are
/// prefixed with `-` instead of using two's complement. This value must be sent to the Mojang
/// session server to verify that the client performed the encryption handshake.
pub fn minecraft_hash(server_id: &str, shared_secret: &[u8], encoded_public: &[u8]) -> String {
    // create a new hasher instance, take the digest and convert it to Minecraft's format
    let mut hasher = Sha1::new();
    hasher.update(server_id);
    hasher.update(shared_secret);
    hasher.update(encoded_public);
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
