use crate::cookie::Cookie;
use passage_adapters::authentication::ProfileProperty;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use uuid::Uuid;

/// The auth cookie key.
pub const AUTH_COOKIE_KEY: &str = "passage:authentication";

/// The [`AuthCookie`] holds the authenticated player information for the connecting client. The cookie
/// is signed using a shared secret. As such, servers the client connects to may skip any additional
/// authentication and use this instead. It also may include the transfer target for further security.
///
/// Generally, the cookie should be checked for expiry to prevent reply attacks. Use the cookie creation
/// time to check.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthCookie {
    /// The time at which the auth cookie was created.
    pub timestamp: u64,

    /// The address of the client that (initially) connected.
    pub client_addr: SocketAddr,

    /// The (authenticated) name of the player.
    pub user_name: String,

    /// The (authenticated) id of the player.
    pub user_id: Uuid,

    /// The (optional) target the client is transferred to. This will be set by passage but may be
    /// omitted by other tools.
    pub target: Option<String>,

    /// The (authenticated) profile properties of the player.
    pub profile_properties: Vec<ProfileProperty>,

    /// Any additional system-specific (secured) information. This includes the OpenTelemetry tracing
    /// information.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl Cookie for AuthCookie {
    const KEY: &'static str = AUTH_COOKIE_KEY;
}
