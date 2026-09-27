use crate::cookie::{Cookie, CookieError};
use passage_core::wire::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::bytes::{BufMut, BytesMut};
use uuid::Uuid;

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

/// The [`SessionCookie`] holds any additional session information about the client. This information
/// is not signed and may be tampered with by the client. Instead, it is meant to store additional
/// information supplementing the [`AuthCookie`](super::auth::AuthCookie) without the additional signature bytes and being
/// configurable without the need for the signing secret.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionCookie {
    /// The ID of the session.
    pub id: Uuid,

    /// The address of the server, the client (initially) connected to.
    pub server_address: String,

    /// The port of the server, the client (initially) connected to.
    pub server_port: u16,

    /// Any additional system-specific (unsecured) information. This includes the OpenTelemetry tracing
    /// information.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl Cookie for SessionCookie {
    const KEY: &'static str = SESSION_COOKIE_KEY;

    fn encode(&self, _: Option<&[u8]>) -> Result<Bytes, CookieError> {
        let mut bytes = BytesMut::with_capacity(64);
        serde_json::to_writer((&mut bytes).writer(), self)?;
        Ok(bytes.freeze())
    }

    fn decode(_: Option<&[u8]>, signed: &[u8]) -> Result<Option<Self>, CookieError> {
        Ok(Some(serde_json::from_slice(signed)?))
    }
}
