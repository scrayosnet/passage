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
//! The application configuration can be created by using [Config::new]. This loads/overrides the
//! configuration fields layer-by-layer.
//!
//! ```rs
//! let config: Config = Config::new()?;
//! ```

use crate::adapter;
use adapter::status::Protocol;
use config::{
    ConfigError, Environment, File, FileFormat, FileStoredFormat, Format, Map, Value, ValueKind,
};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use tracing::warn;
use uuid::Uuid;

/// [Localization] holds all localizable messages of the application.
#[derive(Debug, Clone, Deserialize)]
pub struct Localization {
    /// The locale to be used in case the client locale is unknown or unsupported.
    pub default_locale: String,

    /// The localizable messages.
    pub messages: HashMap<String, HashMap<String, String>>,
}

impl Localization {
    pub fn localize_default(&self, key: &str, params: &[(&'static str, String)]) -> String {
        self.localize(&self.default_locale, key, params)
    }

    pub fn localize(&self, locale: &str, key: &str, params: &[(&'static str, String)]) -> String {
        let locales = [
            locale,
            &locale[..2],
            &self.default_locale,
            &self.default_locale[..2],
        ];

        let mut locale_messages = None;
        for locale in locales.iter() {
            locale_messages = self.messages.get(*locale);
            if locale_messages.is_some() {
                break;
            }
        }

        let Some(locale_messages) = locale_messages else {
            warn!(locales = ?locales, "cannot find locales");
            return key.to_string();
        };

        let Some(template) = locale_messages.get(key) else {
            return key.to_string();
        };

        let mut message = template.clone();
        for (param_key, param_val) in params {
            message = message.replace(param_key, param_val);
        }
        message
    }
}

impl Default for Localization {
    fn default() -> Self {
        Self {
            default_locale: "en_US".to_string(),
            messages: HashMap::new(),
        }
    }
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

/// [Status] hold the status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    /// Adapter to retrieve the results.
    pub adapter: String,

    /// The config for the fixed status.
    pub fixed: Option<FixedStatus>,

    /// The config for the grpc status.
    pub grpc: Option<GrpcStatus>,

    /// The config for the mongodb status.
    pub mongodb: Option<MongodbStatus>,
}

/// [FixedStatus] hold the fixed status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedStatus {
    pub name: String,
    pub description: Option<String>,
    pub favicon: Option<String>,
    pub enforces_secure_chat: Option<bool>,
    pub preferred_version: Protocol,
    pub min_version: Protocol,
    pub max_version: Protocol,
}

/// [GrpcStatus] hold the gRPC status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcStatus {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [MongodbStatus] hold the mongodb status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MongodbStatus {
    /// The address of the mongodb adapter server.
    pub address: String,

    /// The database of the mongodb adapter server.
    pub database: String,

    /// The collection of the mongodb adapter server.
    pub collection: String,

    /// The filter on the collection to get the document(s).
    pub filter: String,

    /// The field path of the filtered document(s).
    pub field_path: Vec<String>,
}

/// [Resourcepack] hold the resourcepack configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Resourcepack {
    /// Adapter to retrieve the results.
    pub adapter: String,

    /// The config for the fixed resourcepack.
    pub fixed: Option<FixedResourcepack>,

    /// The config for the grpc resourcepack.
    pub grpc: Option<GrpcResourcepack>,

    /// The config for the impackable resourcepack.
    pub impackable: Option<ImpackableResourcepack>,
}

/// [FixedResourcepack] hold the resourcepack configuration for a fixed set of packs.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedResourcepack {
    /// The resource packs that should be served.
    pub packs: Vec<adapter::resourcepack::Resourcepack>,
}

/// [GrpcResourcepack] hold the gRPC resourcepack configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcResourcepack {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [ImpackableResourcepack] hold the impackable resourcepack configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ImpackableResourcepack {
    /// The base URL of the impackable resourcepack server.
    pub base_url: String,

    /// The username to authenticate against the query endpoint.
    pub username: String,

    /// The username to authenticate against the query endpoint.
    pub password: String,

    /// The channel that should be filtered for the query endpoint.
    pub channel: String,

    /// The UUID that should be used to identify the resourcepack.
    pub uuid: Uuid,

    /// Whether the download of the resourcepack should be forced.
    pub forced: bool,

    /// The cache duration in seconds to store the queried version.
    pub cache_duration: u64,
}

/// [TargetDiscovery] hold the target discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetDiscovery {
    /// Adapter to retrieve the results.
    pub adapter: String,

    /// The config for the fixed target discovery configuration.
    pub fixed: Option<FixedTargetDiscovery>,

    /// The config for the grpc target discovery configuration.
    pub grpc: Option<GrpcTargetDiscovery>,

    /// The config for the agones target discovery configuration.
    pub agones: Option<AgonesTargetDiscovery>,
}

/// [FixedTargetDiscovery] hold the target discovery configuration for a fixed target.
#[derive(Debug, Clone, Deserialize)]
pub struct FixedTargetDiscovery {
    /// The resource packs that should be served.
    pub targets: Vec<adapter::target_selection::Target>,
}

/// [GrpcTargetDiscovery] hold the gRPC target discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcTargetDiscovery {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [AgonesTargetDiscovery] hold the agones target discovery configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct AgonesTargetDiscovery {
    /// The namespace to search for agones game servers.
    pub namespace: String,
}

/// [TargetStrategy] hold the target strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TargetStrategy {
    /// Adapter to retrieve the results.
    pub adapter: String,

    /// The config for the grpc target strategy.
    pub grpc: Option<GrpcTargetStrategy>,

    /// The config for the player fill target strategy.
    pub player_fill: Option<PlayerFillTargetStrategy>,
}

/// [GrpcTargetStrategy] hold the gRPC target strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GrpcTargetStrategy {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [PlayerFillTargetStrategy] hold the player fill target strategy configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct PlayerFillTargetStrategy {
    /// The name of the field that stores the player amount.
    pub field: String,
    /// The number of players that will be filled at maximum.
    pub max_players: u32,
}

/// [Config] holds all configuration for the application. I.g. one immutable instance is created
/// on startup and then shared among the application components.
///
/// If both the grpc and rest server are disabled, the application will exit immediately after startup
/// with status ok.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// The metrics server address.
    pub metrics_address: SocketAddr,

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

    /// The status (ping) configuration.
    pub status: Status,

    /// The resourcepack configuration.
    pub resourcepack: Resourcepack,

    /// The target discovery configuration.
    pub target_discovery: TargetDiscovery,

    /// The target strategy configuration.
    pub target_strategy: TargetStrategy,

    /// The localization configuration.
    pub localization: Localization,
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
