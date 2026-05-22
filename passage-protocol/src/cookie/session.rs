use crate::cookie::Cookie;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

/// The [`SessionCookie`] holds any additional session information about the client. This information
/// is not signed and may be tampered with by the client. Instead, it is ment to store additional
/// information supplementing the [`AuthCookie`] without the additional signature bytes and being
/// configurable without the need for the signing secret.
#[derive(Debug, Serialize, Deserialize)]
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
}
