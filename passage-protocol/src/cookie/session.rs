use crate::cookie::Cookie;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionCookie {
    pub id: Uuid,
    pub server_address: String,
    pub server_port: u16,
    // the extra data holds any system-specific (unsecured) user information and OpenTelemetry data
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl Cookie for SessionCookie {
    const KEY: &'static str = SESSION_COOKIE_KEY;
}
