//! This module defines and handles the configuration and settings for passage.
//!
//! The configuration values have an impact on the responses and the behavior of passage and come with their own
//! defaults, allowing for startup with zero explicit configuration. Instead, all configuration is implicit, falling
//! back to sensible defaults. The configuration is also used as the immutable state of the application within passage,
//! so that all tasks can use the supplied values.

use std::net::SocketAddr;

/// The default socket address touple and port that passage should listen on.
pub const DEFAULT_ADDRESS: ([u8; 4], u16) = ([0, 0, 0, 0], 25565);

/// The default timeout length that should be used if no specific header is set.
pub const DEFAULT_TIMEOUT_SECS: f64 = 120.0;

/// The default key length for the cryptographic authentication key pair.
pub const DEFAULT_KEY_LENGTH: u32 = 1024;

/// `AppState` contains various, shared resources for the state of the application.
///
/// The state of the application can be shared across all requests to benefit from their caching, resource consumption
/// and configuration. The access is handled through passage, allowing for multiple threads to use the same resource
/// without any problems regarding thread safety.
#[derive(Debug, Clone)]
pub struct AppState {
    /// The network address that should be used to bind the HTTP server for connection requests.
    pub address: SocketAddr,
    /// The timeout in fractional seconds that is used for connection timeouts.
    pub timeout: f64,
    /// The length of the cryptographic key pair for authentication.
    pub key_length: u32,
}

impl AppState {
    /// Creates a new [`AppState`] from the supplied configuration parameters that can be used in passage.
    #[must_use]
    pub const fn new(
        address: SocketAddr,
        timeout: f64,
        key_length: u32,
    ) -> Self {
        Self {
            address,
            timeout,
            key_length,
        }
    }
}
