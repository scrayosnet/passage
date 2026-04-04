use crate::cookie::Cookie;
use passage_adapters::authentication::ProfileProperty;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

/// The auth cookie key.
pub const AUTH_COOKIE_KEY: &str = "passage:authentication";

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

impl Cookie for AuthCookie {
    const KEY: &'static str = AUTH_COOKIE_KEY;
}
