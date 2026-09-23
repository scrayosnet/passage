use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

pub mod auth;
pub mod error;
pub mod session;

pub use auth::*;
pub use error::*;
use passage_core::wire::Bytes;
pub use session::*;

/// Hmac type, expects 32 Byte hash
pub type HmacSha256 = Hmac<Sha256>;

pub(crate) const HASH_LEN: usize = 32;

pub trait Cookie: Sized {
    const KEY: &'static str;

    /**
     * Encodes the cookie into a [`Bytes`] object. Some cookies require a secret to be present, while
     * others ignore it.
     */
    fn encode(&self, secret: Option<&[u8]>) -> Result<Bytes, CookieError>;

    /**
     * Decodes the cookie from a [`Bytes`] object. Some cookies require a secret to be present, while
     * others ignore it. Returns `None` if the cookie is invalid.
     */
    fn decode(secret: Option<&[u8]>, signed: &[u8]) -> Result<Option<Self>, CookieError>;
}

/// Signs a message with a secret in place. Expects `buf` to be `[32 placeholder bytes][message]`.
/// Computes the HMAC of the message and writes it into the first 32 bytes.
pub fn sign(buf: &mut [u8], secret: &[u8]) {
    let (hash_slot, message) = buf.split_at_mut(HASH_LEN);
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(message);
    hash_slot.copy_from_slice(&mac.finalize().into_bytes());
}

/// Verifies a signed message with a secret. Returns whether the signature is valid, as well as the
/// inner message. Use [`sign`] to create a signed message.
#[must_use]
pub fn verify<'a>(signed: &'a [u8], secret: &[u8]) -> Option<&'a [u8]> {
    if signed.len() < HASH_LEN {
        return None;
    }

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(&signed[HASH_LEN..]);
    mac.verify_slice(&signed[..HASH_LEN]).ok().map(|_| &signed[HASH_LEN..])
}

#[cfg(test)]
mod tests {
    // TODO add tests for sign/verify (happy/invalid_message/invalid_secret)
}
