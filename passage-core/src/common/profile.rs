use crate::ProtocolVersion;
use crate::common::{MAX_PROPERTY_NAME_LEN, MAX_PROPERTY_SIGNATURE_LEN, MAX_PROPERTY_VALUE_LEN};
use crate::wire::{Property, Reader, WireError, Writer};
use bytestring::ByteString;
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
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
#[cfg_attr(feature = "config-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The unique identifier of the Minecraft user profile.
    pub id: Uuid,
    /// The current visual name of the Minecraft user profile.
    // `ByteString` is a string with a cheaper clone, and that is all a schema has to know.
    #[cfg_attr(feature = "config-schema", schemars(with = "String"))]
    pub name: ByteString,
    /// The currently assigned properties of the Minecraft user profile.
    #[serde(default)]
    pub properties: Vec<ProfileProperty>,
    /// The pending imposed moderative actions of the Minecraft user profile.
    #[serde(default)]
    #[cfg_attr(feature = "config-schema", schemars(with = "Vec<String>"))]
    pub profile_actions: Vec<ByteString>,
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
#[cfg_attr(feature = "config-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ProfileProperty {
    /// The unique, identifiable name of the profile property.
    #[cfg_attr(feature = "config-schema", schemars(with = "String"))]
    pub name: ByteString,
    /// The base64 encoded value of the profile property.
    #[cfg_attr(feature = "config-schema", schemars(with = "String"))]
    pub value: ByteString,
    /// The base64 encoded signature of the profile property.
    /// Only provided if `?unsigned=false` is appended to url
    #[cfg_attr(feature = "config-schema", schemars(with = "Option<String>"))]
    pub signature: Option<ByteString>,
}

/// On the wire a property is its name, its value, and the signature the session server put on it --
/// which the client only gets if it was asked for one, hence the optional.
impl Property for ProfileProperty {
    const NAME: &'static str = "profile_property";

    fn decode(r: &mut Reader, _: ProtocolVersion, _: &'static str) -> Result<Self, WireError> {
        Ok(Self {
            name: r.string("name", MAX_PROPERTY_NAME_LEN)?,
            value: r.string("value", MAX_PROPERTY_VALUE_LEN)?,
            signature: r.optional("signature", |r| {
                r.string("signature", MAX_PROPERTY_SIGNATURE_LEN)
            })?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("name", &self.name)?;
        w.string("value", &self.value)?;
        w.optional(self.signature.as_deref(), |w, signature| {
            w.string("signature", signature)
        })?;
        Ok(())
    }
}

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
    // TODO replace this with custom implementation. Adding a create just for this call is excessive.
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
