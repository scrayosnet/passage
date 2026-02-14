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
//! Environment variables have the format `[ENV_PREFIX]_[field]_[sub_field]` where `ENV_PREFIX` is
//! an environment variable defaulting to `PASSAGE`. That means, the nested config field `cache.redis.enabled`
//! can be overwritten by the environment variable `PASSAGE_CACHE_REDIS_ENABLED`.
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
//! The application configuration can be created by using [`Config::new`]. This loads/overrides the
//! configuration fields layer-by-layer.
//!
//! ```rs
//! let config: Config = Config::new()?;
//! ```

use config::{
    ConfigError, Environment, File, FileFormat, FileStoredFormat, Format, Map, Value, ValueKind,
};
use passage_adapters::authentication::Profile;
use passage_adapters::{Protocol, Target};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;

/// [`Config`] holds all configuration for the application. I.g. one immutable instance is created
/// on startup and then shared among the application components.
///
/// If both the grpc and rest server are disabled, the application will exit immediately after startup
/// with status ok.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// The sentry configuration (disabled if empty).
    pub sentry: Option<Sentry>,

    /// The OpenTelemetry configuration (disabled if empty).
    #[serde(default, alias = "opentelemetry")]
    pub otel: OpenTelemetry,

    /// The rate limiter config (disabled if empty).
    #[serde(alias = "ratelimiter")]
    pub rate_limiter: Option<RateLimiter>,

    /// The PROXY protocol config (disabled if empty).
    #[serde(alias = "proxyprotocol")]
    pub proxy_protocol: Option<ProxyProtocol>,

    /// The network address that should be used to bind the HTTP server for connection requests.
    pub address: SocketAddr,

    /// The timeout in seconds that is used for connection timeouts.
    pub timeout: u64,

    /// The auth cookie secret, disabled if empty.
    #[serde(alias = "authsecret")]
    pub auth_secret: Option<String>,

    /// The adapters configuration.
    pub adapters: Adapters,
}

/// [`Sentry`] hold the sentry configuration. The release is automatically inferred from cargo.
#[derive(Debug, Clone, Deserialize)]
pub struct Sentry {
    /// Whether sentry should have debug enabled.
    pub debug: bool,

    /// The sentry environment of the application.
    pub environment: String,

    /// The address of the sentry instance. This can either be the official sentry or a self-hosted instance.
    pub address: String,
}

/// [`OpenTelemetry`] hold the OpenTelemetry configuration. The release is automatically inferred from cargo.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct OpenTelemetry {
    /// The OpenTelemetry environment of the application.
    pub environment: String,

    /// The traces configuration (disabled if empty).
    pub traces: Option<OpenTelemetryEndpoint>,

    /// The traces configuration (disabled if empty).
    pub metrics: Option<OpenTelemetryEndpoint>,
}

/// [`OpenTelemetryEndpoint`] hold the OpenTelemetry configuration for a specific endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenTelemetryEndpoint {
    /// The address of the http/protobuf.
    pub address: String,

    /// The base64 basic auth token.
    pub token: String,
}

/// [`RateLimiter`] hold the connection rate limiting configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimiter {
    /// Duration in seconds.
    pub duration: u64,

    /// Maximum amount of connections per duration.
    pub size: usize,
}

/// [`ProxyProtocol`] hold the PROXY protocol configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyProtocol {
    /// Whether to allow V1 headers
    pub allow_v1: bool,

    /// Whether to allow V2 headers
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

/// [`Adapters`] holds the adapter configurations.
#[derive(Debug, Clone, Deserialize)]
pub struct Adapters {
    /// The status (ping) adapter configuration.
    pub status: StatusAdapter,

    /// The discovery adapter configuration.
    pub discovery: DiscoveryAdapter,

    /// The filter adapter configuration.
    pub filter: Vec<FilterAdapter>,

    /// The strategy adapter configuration.
    pub strategy: StrategyAdapter,

    /// The authentication adapter configuration.
    pub authentication: AuthenticationAdapter,

    /// The localization adapter configuration.
    pub localization: LocalizationAdapter,
}

/// [`StatusAdapter`] hold the status adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusAdapter {
    Fixed(FixedStatus),
    Grpc(GrpcStatus),
    Http(HttpStatus),
}

/// [`FixedStatus`] hold the fixed status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedStatus {
    /// The name of the server.
    pub name: String,

    /// The description of the server.
    pub description: Option<String>,

    /// The favicon of the server.
    pub favicon: Option<String>,

    /// Whether the server requires secure chat.
    #[serde(alias = "enforcessecurechat")]
    pub enforces_secure_chat: Option<bool>,

    /// The preferred protocol version of the server.
    #[serde(alias = "preferredversion")]
    pub preferred_version: Protocol,

    /// The minimum protocol version supported by the server.
    #[serde(alias = "minversion")]
    pub min_version: Protocol,

    /// The maximum protocol version supported by the server.
    #[serde(alias = "maxversion")]
    pub max_version: Protocol,
}

/// [`GrpcStatus`] hold the gRPC status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcStatus {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`HttpStatus`] hold the http status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct HttpStatus {
    /// The address of the http adapter server.
    pub address: String,

    /// The cache duration in seconds to store the queried status.
    #[serde(alias = "cacheduration")]
    pub cache_duration: u64,
}

/// [`DiscoveryAdapter`] hold the discovery adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryAdapter {
    Fixed(FixedDiscovery),
    Agones(AgonesDiscovery),
    Grpc(GrpcDiscovery),
}

/// [`FixedDiscovery`] hold the fixed discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedDiscovery {
    /// The targets that should be served by the discovery adapter.
    pub targets: Vec<Target>,
}

/// [`AgonesDiscovery`] hold the agones discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AgonesDiscovery {
    /// The namespace to apply to the watcher.
    pub namespace: Option<String>,

    /// The label selector to apply to the watcher.
    pub label_selector: Option<String>,

    /// The field selector to apply to the watcher.
    pub field_selector: Option<String>,
}

/// [`GrpcDiscovery`] hold the gRPC discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcDiscovery {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`FilterAdapter`] hold the filter adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterAdapter {
    Fixed(FixedFilter),
}

/// [`FixedFilter`] hold the fixed filter configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedFilter {
    // TODO add some logic here!
}

/// [`StrategyAdapter`] hold the strategy adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyAdapter {
    Fixed(FixedStrategy),
    PlayerFill(PlayerFillStrategy),
    Grpc(GrpcStrategy),
}

/// [`FixedStrategy`] hold the fixed strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedStrategy {
    // TODO add some logic here!
}

/// [`FixedStrategy`] hold the fixed strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerFillStrategy {
    /// The name of the field that stores the player amount.
    pub field: String,

    /// The number of players that will be filled at maximum.
    #[serde(alias = "maxplayers")]
    pub max_players: u32,
}

/// [`GrpcStrategy`] hold the gRPC strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcStrategy {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`AuthenticationAdapter`] hold the authentication adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthenticationAdapter {
    Fixed(FixedAuthentication),
    Mojang(MojangAuthentication),
}

/// [`FixedAuthentication`] hold the fixed authentication configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedAuthentication {
    /// The fixed profile that should be used for authentication.
    pub profile: Profile,
}

/// [`MojangAuthentication`] hold the mojang authentication configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MojangAuthentication {
    /// The server id passed to the Mojang authentication server.
    #[serde(default, alias = "serverid")]
    pub server_id: String,
}

/// [`LocalizationAdapter`] hold the localization adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalizationAdapter {
    Fixed(FixedLocalization),
}

/// [`FixedLocalization`] hold the fixed localization configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedLocalization {
    /// The locale to be used in case the client locale is unknown or unsupported.
    #[serde(alias = "defaultlocale")]
    pub default_locale: String,

    /// The localizable messages.
    pub messages: HashMap<String, HashMap<String, String>>,
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
            // e.g. `PASSAGE_DEBUG=1` would set the `debug` key, on the other hand,
            // `PASSAGE_CACHE_REDIS_ENABLED=1` would enable the redis cache.
            .add_source(Environment::with_prefix(&env_prefix).separator("_"))
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
