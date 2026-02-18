use hmac::{Hmac, Mac};
use opentelemetry::trace::SpanContext;
use opentelemetry::{SpanId, TraceFlags, TraceId};
use passage_adapters::authentication::ProfileProperty;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

/// The auth cookie key.
pub const AUTH_COOKIE_KEY: &str = "passage:authentication";

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

/// The default expiry of the auth cookie (6 hours).
pub const AUTH_COOKIE_EXPIRY_SECS: u64 = 6 * 60 * 60;

/// Hmac type, expects 32 Byte hash
pub type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthCookie {
    pub timestamp: u64,
    pub client_addr: SocketAddr,
    pub user_name: String,
    pub user_id: Uuid,
    pub target: Option<String>,
    pub profile_properties: Vec<ProfileProperty>,
    // the extra data holds any system-specific (secured) user information
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionCookie {
    pub id: Uuid,
    pub server_address: String,
    pub server_port: u16,
    #[serde(default)]
    pub trace_id: Option<String>,
}

impl SessionCookie {
    pub fn span_cx(&self) -> Option<SpanContext> {
        // get trace id and parse
        let Some(trace_id) = &self.trace_id else {
            trace!("no trace id set in session cookie");
            return None;
        };

        let Ok(trace_id) = TraceId::from_hex(trace_id) else {
            trace!("failed to parse trace id from session cookie");
            return None;
        };

        // create span context from trace id
        Some(SpanContext::new(
            trace_id,
            SpanId::INVALID,
            TraceFlags::SAMPLED,
            true,
            Default::default(),
        ))
    }
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
pub fn verify<'a>(signed: &'a [u8], secret: &[u8]) -> (bool, &'a [u8]) {
    if signed.len() < 32 {
        return (false, b"");
    }

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(&signed[32..]);
    let ok = mac.verify_slice(&signed[..32]).is_ok();
    (ok, &signed[32..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify() {
        let message = b"justchunks";
        let secret = b"secret";

        let signed = sign(message, secret);
        let (ok, verified) = verify(&signed, secret);

        assert!(ok);
        assert_eq!(message, verified);
    }

    #[test]
    fn sign_verify_invalid_secret() {
        let message = b"justchunks";
        let secret1 = b"secret1";
        let secret2 = b"secret2";

        let signed = sign(message, secret1);
        let (ok, _) = verify(&signed, secret2);

        assert!(!ok);
    }

    #[test]
    fn sign_verify_invalid_message() {
        let message = b"justchunks";
        let secret = b"secret";

        let (ok, _) = verify(message, secret);

        assert!(!ok);
    }
}
