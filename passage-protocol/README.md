# Passage Protocol

Contains an opinionated implementation of the Minecraft protocol. It is configured using routes which
match connecting clients and handle them using the route-specific adapters.

Passage uses two cookies for its connections. The first is an **authentication cookie** that encodes
the authenticated player information. This cookie is signed using a shared secret such that any server
the player is transferred to, may skip any additional authentication requests. The second cookie is
a **session cookie** that holds any additional session information such as the OpenTelemetry tracing
information. It is NOT signed and can be easily tampered with by any client.

```rs
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
```
