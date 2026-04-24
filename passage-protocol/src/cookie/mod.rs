use hmac::{Hmac, KeyInit, Mac};
use passage_packets::configuration::clientbound::StoreCookiePacket;
use passage_packets::login::serverbound::CookieResponsePacket;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub mod auth;
pub mod error;
pub mod session;

pub use auth::*;
pub use error::*;
pub use session::*;

/// Hmac type, expects 32 Byte hash
pub type HmacSha256 = Hmac<Sha256>;

pub trait Cookie: Sized {
    const KEY: &'static str;
}

/// Signs a message with a secret. Returns the signed message. Use [`verify`] to verify and destruct
/// the signed message.
#[must_use]
pub fn sign(message: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(message);
    let hash = mac.finalize().into_bytes();

    // Build the final binary format (hash is always 32 byte)
    let mut output = Vec::with_capacity(32 + message.len());
    output.extend_from_slice(&hash);
    output.extend_from_slice(message);

    output
}

/// Verifies a signed message with a secret. Returns whether the signature is valid, as well as the
/// inner message. Use [`sign`] to create a signed message.
#[must_use]
pub fn verify<'a>(signed: &'a [u8], secret: &[u8]) -> Option<&'a [u8]> {
    if signed.len() < 32 {
        return None;
    }

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(&signed[32..]);
    mac.verify_slice(&signed[..32]).ok().map(|_| &signed[32..])
}

pub trait CookieEncodeExt: Sized {
    fn encode<T: Cookie + Serialize>(cookie: &T) -> Result<Self, Error>;

    fn encode_signed<T: Cookie + Serialize>(secret: &[u8], cookie: &T) -> Result<Self, Error>;
}

impl CookieEncodeExt for StoreCookiePacket {
    fn encode<T: Cookie + Serialize>(cookie: &T) -> Result<Self, Error> {
        let cookie_bytes = serde_json::to_vec(cookie)?;
        Ok(StoreCookiePacket {
            key: T::KEY.to_owned(),
            payload: cookie_bytes,
        })
    }

    fn encode_signed<T: Cookie + Serialize>(secret: &[u8], cookie: &T) -> Result<Self, Error> {
        let cookie_bytes = serde_json::to_vec(cookie)?;
        let message = sign(&cookie_bytes, secret);
        Ok(StoreCookiePacket {
            key: T::KEY.to_owned(),
            payload: message,
        })
    }
}

pub trait CookieDecodeExt: Sized {
    fn decode<'de, T: Cookie + Deserialize<'de>>(&'de self) -> Result<Option<T>, Error>;

    fn decode_verified<'de, T: Cookie + Deserialize<'de>>(
        &'de self,
        secret: &[u8],
    ) -> Result<Option<T>, Error>;
}

impl CookieDecodeExt for CookieResponsePacket {
    fn decode<'de, T: Cookie + Deserialize<'de>>(&'de self) -> Result<Option<T>, Error> {
        let Some(message) = &self.payload else {
            return Ok(None);
        };
        Ok(serde_json::from_slice(message)?)
    }

    fn decode_verified<'de, T: Cookie + Deserialize<'de>>(
        &'de self,
        secret: &[u8],
    ) -> Result<Option<T>, Error> {
        let Some(message) = &self.payload else {
            return Ok(None);
        };
        let Some(message) = verify(message, secret) else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice(message)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify() {
        let message = b"justchunks";
        let secret = b"secret";

        let signed = sign(message, secret);
        let verified = verify(&signed, secret);

        assert_eq!(Some(message.as_slice()), verified);
    }

    #[test]
    fn sign_verify_invalid_secret() {
        let message = b"justchunks";
        let secret1 = b"secret1";
        let secret2 = b"secret2";

        let signed = sign(message, secret1);
        let verified = verify(&signed, secret2);

        assert_eq!(None, verified);
    }

    #[test]
    fn sign_verify_invalid_message() {
        let message = b"justchunks";
        let secret = b"secret";

        let verified = verify(message, secret);

        assert_eq!(None, verified);
    }
}
