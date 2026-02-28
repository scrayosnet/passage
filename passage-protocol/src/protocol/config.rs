use passage_packets::VarInt;
use serde::Deserialize;

/// The max packet length in bytes. Larger packets are rejected.
pub const DEFAULT_MAX_PACKET_LENGTH: VarInt = 10_000;

/// The default auth cookie expiry time in seconds.
pub const DEFAULT_AUTH_COOKIE_EXPIRY: u64 = 6 * 60 * 60;

/// The default timeout for a single connection in seconds.
pub const DEFAULT_CONNECTION_TIMEOUT: u64 = 120;

#[derive(Debug, Clone)]
pub struct Config {
    /// The auth secret used to sign and verify auth cookies.
    pub auth_secret: Option<String>,

    /// The max packet length in bytes. Larger packets are rejected.
    pub max_packet_length: VarInt,

    /// The auth cookie expiry time in seconds.
    pub auth_cookie_expiry: u64,

    /// Whether to enable the proxy protocol.
    pub proxy_protocol: Option<ProxyProtocol>,

    /// The timeout for a single connection in seconds.
    pub connection_timeout: u64,
}

impl Config {
    pub fn with_auth_secret(mut self, auth_secret: Option<String>) -> Self {
        self.auth_secret = auth_secret;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auth_secret: None,
            max_packet_length: DEFAULT_MAX_PACKET_LENGTH,
            auth_cookie_expiry: DEFAULT_AUTH_COOKIE_EXPIRY,
            proxy_protocol: None,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProxyProtocol {
    /// Whether to allow V1 headers
    #[serde(alias = "allowv1")]
    pub allow_v1: bool,

    /// Whether to allow V2 headers
    #[serde(alias = "allowv2")]
    pub allow_v2: bool,
}

impl Default for ProxyProtocol {
    fn default() -> Self {
        Self {
            allow_v1: true,
            allow_v2: true,
        }
    }
}
