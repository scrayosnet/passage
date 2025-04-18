//! The config module defines the application configuration. It is based on [config], a layered
//! configuration system for Rust applications (with strong support for 12-factor applications).
//!
//! # Layers
//!
//! The configuration consists of multiple layers. Upper layers overwrite lower layer configurations
//! (e.g. environment variables overwrite the default configuration).
//!
//! ## Layer 1 (Environment variables) \[optional\]
//!
//! The environment variables are the top most layer. They can be used to overwrite any previous configuration.
//! Environment variables have the format `[ENV_PREFIX]__[field]__[sub_field]` where `ENV_PREFIX` is
//! an environment variable defaulting to `PASSAGE`. That means, the nested config field `cache.redis.enabled`
//! can be overwritten by the environment variable `PASSAGE__CACHE__REDIS__ENABLED`.
//!
//! ## Layer 2 (Auth Secret File) \[optional\]
//!
//! The next layer is just for setting the auth cookie secret. It reads a single file as the key. Set
//! the location using the `AUTH_SECRET_FILE` environment variable, defaulting to `config/auth_secret`.
//!
//! ## Layer 3 (Custom configuration) \[optional\]
//!
//! The next layer is an optional configuration file intended to be used by deployments and local testing. The file
//! location can be configured using the `CONFIG_FILE` environment variable, defaulting to `config/config`.
//! It can be of any file type supported by [config] (e.g. `config/config.toml`). The file should not be
//! published by git as its configuration is context dependent (e.g. local/cluster) and probably contains
//! secrets.
//!
//! ## Layer 4 (Default configuration)
//!
//! The default configuration provides default value for all config fields. It is loaded from
//! `config/default.toml` at compile time.
//!
//! # Usage
//!
//! The application configuration can be created by using [Config::new]. This loads/overrides the
//! configuration fields layer-by-layer.
//!
//! ```rs
//! let config: Config = Config::new()?;
//! ```

use crate::status;
use config::{
    ConfigError, Environment, File, FileFormat, FileStoredFormat, Format, Map, Value, ValueKind,
};
use serde::Deserialize;
use std::env;
use std::net::SocketAddr;

/// [Metrics] holds the metrics service configuration. The metrics service is part of the rest server.
/// The rest server will be, if not already so, implicitly enabled if the metrics service is enabled.
/// If enabled, it is exposed at the rest server at `/metrics`.
///
/// Metrics will always be aggregated by the application. This option is only used to expose the metrics
/// service. The service supports basic auth that can be enabled. Make sure to override the default
/// username and password in that case.
#[derive(Debug, Clone, Deserialize)]
pub struct Metrics {
    /// Whether the metrics service should be enabled.
    pub enabled: bool,

    /// Whether the metrics service should use basic auth.
    pub auth_enabled: bool,

    /// The basic auth username. Override default configuration if basic auth is enabled.
    pub username: String,

    /// The basic auth password. Override default configuration if basic auth is enabled.
    pub password: String,
}

/// [Sentry] hold the sentry configuration. The release is automatically inferred from cargo.
#[derive(Debug, Clone, Deserialize)]
pub struct Sentry {
    /// Whether sentry should be enabled.
    pub enabled: bool,

    /// Whether sentry should have debug enabled.
    pub debug: bool,

    /// The address of the sentry instance. This can either be the official sentry or a self-hosted instance.
    /// The address has to bes event if sentry is disabled. In that case, the address can be any non-nil value.
    pub address: String,

    /// The environment of the application that should be communicated to sentry.
    pub environment: String,
}

/// [RateLimiter] hold the connection rate limiting configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiter {
    pub enabled: bool,
    /// Duration in seconds
    pub duration: u64,
    pub size: usize,
}

/// [Protocol] hold the protocol limitation and
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Protocol {
    pub preferred: status::Protocol,
    pub min: status::Protocol,
    pub max: status::Protocol,
}

/// [Status] hold the status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    /// Adapter to retrieve the results.
    pub adapter: String,

    /// The config for the fixed status.
    pub fixed: Option<FixedStatus>,
}

/// [FixedStatus] hold the fixed status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedStatus {
    pub name: String,
    pub description: Option<String>,
    pub favicon: Option<String>,
    pub enforces_secure_chat: Option<bool>,
}

/// [Resourcepack] hold the resourcepack configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Resourcepack {
    /// Adapter to retrieve the results.
    pub adapter: String,

    pub fixed: Option<FixedResourcepack>,
}

/// [Resourcepack] hold the resourcepack configuration for a fixed set of packs.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedResourcepack {
    /// Adapter to retrieve the results.
    pub packs: Vec<crate::adapter::resourcepack::Resourcepack>,
}

/// [Target] hold the target discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    /// Adapter to retrieve the results.
    pub adapter: String,

    pub fixed: Option<FixedTarget>,
}

/// [FixedTarget] hold the target discovery configuration for a fixed target.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedTarget {
    pub identifier: String,
    pub address: SocketAddr,
}

/// [TargetStrategy] hold the target strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetStrategy {
    /// Adapter to retrieve the results.
    pub adapter: String,
}

/// [Config] holds all configuration for the application. I.g. one immutable instance is created
/// on startup and then shared among the application components.
///
/// If both the grpc and rest server are disabled, the application will exit immediately after startup
/// with status ok.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// The metrics configuration. The metrics service is part of the [RestServer].
    pub metrics: Metrics,

    /// The sentry configuration.
    pub sentry: Sentry,

    /// The rate limiter config.
    pub rate_limiter: RateLimiter,

    /// The network address that should be used to bind the HTTP server for connection requests.
    pub address: SocketAddr,

    /// The timeout in seconds that is used for connection timeouts.
    pub timeout: u64,

    /// The auth cookie secret, disabled if empty.
    pub auth_secret: Option<String>,

    /// The supported protocol version.
    pub protocol: Protocol,

    /// The status (ping) configuration.
    pub status: Status,

    /// The resourcepack configuration.
    pub resourcepack: Resourcepack,

    /// The target discovery configuration.
    pub target: Target,

    /// The target strategy configuration.
    pub target_strategy: TargetStrategy,
}

impl Config {
    /// Creates a new application configuration as described in the [module documentation](crate::config).
    pub fn new() -> Result<Self, ConfigError> {
        // the environment prefix for all `Config` fields
        let env_prefix = env::var("ENV_PREFIX").unwrap_or("passage".into());
        // the path of the custom configuration file
        let config_file = env::var("CONFIG_FILE").unwrap_or("config/config".into());
        let auth_secret_file = env::var("AUTH_SECRET_FILE").unwrap_or("config/auth_secret".into());

        let s = config::Config::builder()
            // load default configuration (embedded at compile time)
            .add_source(File::from_str(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/config/default.toml")),
                FileFormat::Toml,
            ))
            // load custom configuration from file (at runtime)
            .add_source(File::with_name(&config_file).required(false))
            .add_source(File::new(&auth_secret_file, AuthSecretFile).required(false))
            // add in config from the environment (with a prefix of APP)
            // e.g. `PASSAGE__DEBUG=1` would set the `debug` key, on the other hand,
            // `PASSAGE__CACHE__REDIS__ENABLED=1` would enable the redis cache.
            .add_source(Environment::with_prefix(&env_prefix).separator("__"))
            .build()?;

        // you can deserialize (and thus freeze) the entire configuration as
        s.try_deserialize()
    }
}

impl Default for Config {
    fn default() -> Self {
        let s = config::Config::builder()
            // load default configuration (embedded at compile time)
            .add_source(File::from_str(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/config/default.toml")),
                FileFormat::Toml,
            ))
            .build()
            .expect("expected default configuration to be available");

        // you can deserialize (and thus freeze) the entire configuration as
        s.try_deserialize()
            .expect("expected default configuration to be deserializable")
    }
}

#[derive(Debug, Clone)]
pub struct AuthSecretFile;

impl Format for AuthSecretFile {
    fn parse(
        &self,
        uri: Option<&String>,
        text: &str,
    ) -> Result<Map<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let mut result = Map::new();

        result.insert(
            // key has to match config param
            "auth_secret".to_owned(),
            Value::new(uri, ValueKind::String(text.into())),
        );

        Ok(result)
    }
}

impl FileStoredFormat for AuthSecretFile {
    fn file_extensions(&self) -> &'static [&'static str] {
        &[]
    }
}
