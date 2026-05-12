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
//! an environment variable defaulting to `PASSAGE`. That means the nested config field `cache.redis.enabled`
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
//! It can be of any file type supported by [config] (e.g. `config/old_config.toml`). The file should not be
//! published by git as its configuration is context-dependent (e.g. local/cluster) and probably contains
//! secrets.
//!
//! ## Layer 4 (Default configuration)
//!
//! The default configuration provides default values for all config fields. It is defined in the struct.
//!
//! # Usage
//!
//! The application configuration can be created by using [`Config::read`]. This loads/overrides the
//! configuration fields layer-by-layer.
//!
//! ```rs
//! let config: Config = Config::new()?;
//! ```

#![allow(clippy::derivable_impls)]

use crate::metrics::system::DEFAULT_OBSERVE_INTERVAL;
use config::{ConfigError, Environment, File, FileStoredFormat, Format, Map, Value, ValueKind};
use passage_adapters::authentication::Profile;
use passage_adapters::backoff::ExponentialBackoff;
use passage_adapters::{Protocol, Target};
use passage_protocol::config::DEFAULT_CONNECTION_TIMEOUT;
use passage_protocol::connection::{DEFAULT_AUTH_COOKIE_EXPIRY, DEFAULT_MAX_PACKET_LENGTH};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;

macro_rules! hashmap {
    ($($key:expr => $value:expr),* $(,)?) => {{
        let mut map = std::collections::HashMap::new();
        $(map.insert($key.into(), $value.into());)*
        map
    }};
}

/// [`Config`] holds all configuration for the application. I.g. one immutable instance is created
/// on startup and then shared among the application components.
///
/// If both the grpc and rest server are disabled, the application will exit immediately after startup
/// with status ok.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The network address that should be used to bind the HTTP server for connection requests.
    pub address: String,

    /// The timeout in seconds that is used for connection timeouts.
    pub timeout: u64,

    /// The max packet size in bytes accepted by the server.
    #[serde(alias = "maxpacketlength")]
    pub max_packet_length: usize,

    /// The number of seconds until an auth cookie expires.
    #[serde(alias = "authcookieexpiry")]
    pub auth_cookie_expiry: u64,

    /// The interval in seconds at which the system observer should be run.
    #[serde(alias = "systemobserverinterval")]
    pub system_observer_interval: Option<u64>,

    /// The sentry configuration (disabled if empty).
    pub sentry: Option<Sentry>,

    /// The OpenTelemetry configuration (disabled if empty).
    #[serde(alias = "opentelemetry")]
    pub otel: OpenTelemetry,

    /// The rate limiter config (disabled if empty).
    #[serde(alias = "ratelimiter")]
    pub rate_limiter: Option<RateLimiter>,

    /// The PROXY protocol config (disabled if empty).
    #[serde(alias = "proxyprotocol")]
    pub proxy_protocol: Option<ProxyProtocol>,

    /// The auth cookie secret, disabled if empty.
    #[serde(alias = "authsecret")]
    pub auth_secret: Option<String>,

    /// The routes' configuration.
    pub routes: Vec<Routes>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            address: "0.0.0.0:25565".to_string(),
            timeout: DEFAULT_CONNECTION_TIMEOUT,
            system_observer_interval: Some(DEFAULT_OBSERVE_INTERVAL),
            sentry: None,
            otel: OpenTelemetry::default(),
            rate_limiter: None,
            proxy_protocol: None,
            auth_secret: None,
            routes: Default::default(),
            max_packet_length: DEFAULT_MAX_PACKET_LENGTH as usize,
            auth_cookie_expiry: DEFAULT_AUTH_COOKIE_EXPIRY,
        }
    }
}

/// [`Sentry`] hold the sentry configuration. The release is automatically inferred from cargo.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Sentry {
    /// Whether sentry should have debug enabled.
    pub debug: bool,

    /// The sentry environment of the application.
    pub environment: String,

    /// The address of the sentry instance. This can either be the official sentry or a self-hosted instance.
    pub address: String,
}

/// [`OpenTelemetry`] hold the OpenTelemetry configuration. The release is automatically inferred from cargo.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OpenTelemetry {
    /// The OpenTelemetry environment of the application.
    pub environment: String,

    /// The traces configuration (disabled if empty).
    pub traces: Option<OpenTelemetryEndpoint>,

    /// The traces configuration (disabled if empty).
    pub metrics: Option<OpenTelemetryEndpoint>,

    /// The logs configuration (disabled if empty).
    pub logs: Option<OpenTelemetryEndpoint>,
}

/// [`OpenTelemetryEndpoint`] hold the OpenTelemetry configuration for a specific endpoint.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OpenTelemetryEndpoint {
    /// The address of the http/protobuf.
    pub address: String,

    /// The base64 basic auth token.
    pub token: String,
}

/// [`RateLimiter`] hold the connection rate limiting configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RateLimiter {
    /// Duration in seconds.
    pub duration: u64,

    /// Maximum amount of connections per duration.
    pub limit: usize,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            duration: 60,
            limit: 60,
        }
    }
}

/// [`ProxyProtocol`] hold the PROXY protocol configuration.
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

/// [`Routes`] holds the adapter configurations.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Routes {
    /// The hostname the route should serve. Has to be a valid regex.
    #[serde(alias = "servername")]
    pub hostname: String,

    /// The status (ping) adapter configuration.
    pub status: StatusAdapter,

    /// The authentication adapter configuration.
    pub authentication: AuthenticationAdapter,

    /// The localization adapter configuration.
    pub localization: LocalizationAdapter,

    /// The discovery adapter configuration.
    pub discovery: DiscoveryAdapter,
}

/// [`StatusAdapter`] hold the status adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StatusAdapter {
    Fixed(FixedStatus),
    Grpc(GrpcStatus),
    Http(HttpStatus),
}

impl Default for StatusAdapter {
    fn default() -> Self {
        Self::Fixed(FixedStatus::default())
    }
}

/// [`FixedStatus`] hold the fixed status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FixedStatus {
    /// The name of the server.
    #[serde(alias = "servername")]
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

impl Default for FixedStatus {
    fn default() -> Self {
        Self {
            name: "Passage".to_string(),
            description: Some("\"Minecraft Server Transfer Router\"".to_string()),
            favicon: Some("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAEAAAABABAMAAABYR2ztAAAABGdBTUEAALGPC/xhBQAAAAFzUkdCAK7OHOkAAAAeUExURdJDACIiIshABV4oHnQtHKI3FEElILs9DDAjIZMzF3zpuzQAAAIISURBVEjH5ZU7T8MwEMettCUwhpQ0bOG9JoW2jIQWWAMUEBsJr7IV1ErtRkQlxIaQQOLb4sfZPidCfABuiWL/cne++/tCnD+M/FfgbrvXaw9+BXZOAsJsqz/AgJuKt5cTom0NAeFGxB4PQ4It0kBAKOEdcO/W+ELsW6kCfPo6/ubr1mlaBz81HcJTXu224xyVc6jLfWuXJpzAy7EGliRwzmiIYEcaaMD+Oo/3gVMQwCIEEN/MhIsFBDzjrCSBgSsBxLLes6AQ4goHZXbLkkw1EHJgE/VsXznkwJ4ZgXUvMOrAATvHbW9KjwDQvKuGLuryGADQRk1M5byDS0iyP7RiE8iwHkLyeDDNTcCFLKEOK+5hQZz+kKwoICNzZb2HpIKaVSkDmTgX6KFWBmARFGWlJcAllgJcOJJhnvhKyn5S3O7SRQ0kpSzfxvGQu4WbVUpin4wSBDRkc1GZ7IQLQADLoGjcTNphDXiBlDTYq5IQzIdEa9oxNUbUAu63F6j7D8A9U3WKysxN10GsrOdmBAws84VRrjXNLC8CkmjCXXUKIejSFy8CITAR0IEBMGJHkBOk5hh14HZKq9yU46SK5qR0GjteRw2sOQ0sqQjXfJTal5/0rs1rIAMHZ0/8uUHvQCtAsof7L23KS9oKonIOYpRCua7xtPdvLgSz2o9++1/4dzu9bjv9p//NH77UnP1UgYF9AAAAAElFTkSuQmCC".to_string()),
            enforces_secure_chat: Some(true),
            preferred_version: 769,
            min_version: 0,
            max_version: 1_000,
        }
    }
}

/// [`GrpcStatus`] hold the gRPC status (ping) configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcStatus {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`HttpStatus`] hold the http status (ping) configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HttpStatus {
    /// The address of the http adapter server.
    pub address: String,

    /// The cache duration in seconds to store the queried status. Must be greater than zero.
    #[serde(alias = "cacheduration")]
    pub cache_duration: u64,
}

impl Default for HttpStatus {
    fn default() -> Self {
        Self {
            address: "http://localhost:8080".to_string(),
            cache_duration: 60,
        }
    }
}

/// [`DiscoveryAdapter`] hold the discovery (adapter) configuration.
#[derive(Default, Debug, Clone, Deserialize)]
pub struct DiscoveryAdapter {
    /// The discovery adapter configuration to get the initial targets.
    #[serde(flatten)]
    pub adapter: DiscoveryActionAdapter,

    /// Any action to be applied to the discovered targets.
    pub actions: Vec<DiscoveryActionAdapter>,
}

/// [`DiscoveryActionAdapter`] hold the discovery action adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiscoveryActionAdapter {
    #[serde(alias = "fixeddiscovery")]
    FixedDiscovery(FixedDiscovery),
    #[serde(alias = "agonesdiscovery")]
    AgonesDiscovery(AgonesDiscovery),
    #[serde(alias = "grpcdiscovery")]
    GrpcDiscovery(GrpcDiscovery),
    #[serde(alias = "dnsdiscovery")]
    DnsDiscovery(DnsDiscovery),
    Grpc(GrpcDiscoveryAction),
    #[serde(alias = "filter")]
    MetaFilter(MetaFilter),
    #[serde(alias = "playerallowfilter")]
    PlayerAllowFilter(PlayerAllowFilter),
    #[serde(alias = "playerblockfilter")]
    PlayerBlockFilter(PlayerBlockFilter),
    #[serde(alias = "playerfillfilter")]
    PlayerFillStrategy(PlayerFillStrategy),
}

impl Default for DiscoveryActionAdapter {
    fn default() -> Self {
        Self::FixedDiscovery(FixedDiscovery::default())
    }
}

/// [`FixedDiscovery`] hold the fixed discovery configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FixedDiscovery {
    /// The targets that should be served by the discovery adapter.
    pub targets: Vec<Target>,
}

/// [`AgonesDiscovery`] hold the agones discovery configuration. The template values get the following
/// variables as input. Currently, string fields are replaced if they exactly match the variable:
/// - `{{ .Client.ProtocolVersion }}` The client protocol version.
/// - `{{ .Client.ServerAddress }}` The server address (presented by the client).
/// - `{{ .Client.ServerPort }}` The server port (presented by the client).
/// - `{{ .Client.Address }}` The address of the client (with optional proxy protocol).
/// - `{{ .Request.TraceId }}` The opentelemetry trace id of the request.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AgonesDiscovery {
    /// The namespace to apply to the client.
    pub namespace: Option<String>,

    /// The selectors template to apply to the allocation.
    pub selectors: Vec<serde_json::Value>,

    /// The priorities template to apply to the allocation.
    pub priorities: Vec<serde_json::Value>,

    /// The scheduling to apply to the allocation.
    pub scheduling: Option<String>,

    /// The metadata template to apply to the allocation.
    pub metadata: Option<serde_json::Value>,

    /// The exponential backoff configuration.
    pub backoff: ExponentialBackoff,
}

/// [`GrpcDiscovery`] hold the gRPC discovery configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcDiscovery {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`ARecordType`] holds the DNS discovery configuration for A/AAAA records.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ARecordType {
    pub port: u16,
}

/// [`DnsDiscoveryRecordType`] hold the DNS discovery adapter record configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DnsDiscoveryRecordType {
    Srv,
    A(ARecordType),
}

/// [`DnsDiscovery`] hold the DNS discovery configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct DnsDiscovery {
    /// The DNS domain to query (e.g., "_minecraft._tcp.example.com" for SRV or "mc.example.com" for A).
    pub domain: String,

    /// How often to re-query DNS in seconds.
    #[serde(alias = "refreshinterval")]
    pub refresh_interval: u64,

    /// The type of DNS record to query ("srv" or "a").
    #[serde(alias = "recordtype")]
    pub record_type: DnsDiscoveryRecordType,
}

impl Default for DnsDiscovery {
    fn default() -> Self {
        Self {
            domain: String::new(),
            refresh_interval: 30,
            record_type: DnsDiscoveryRecordType::Srv,
        }
    }
}

/// [`MetaFilter`] hold the metadata filter configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MetaFilter {
    /// List of filter rules. All rules must match (AND logic).
    pub rules: Vec<FilterRule>,
}

/// A single filter rule.
#[derive(Debug, Clone, Deserialize)]
pub struct FilterRule {
    /// The metadata key to filter on.
    #[serde(alias = "field")]
    pub key: String,
    /// The operation to perform.
    #[serde(flatten)]
    pub operation: FilterOperation,
}

/// Filter operation to apply to a target field.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", content = "value", rename_all = "snake_case")]
pub enum FilterOperation {
    /// Field must equal the specified value.
    Equals(String),
    /// Field must not equal the specified value.
    #[serde(alias = "notequals")]
    NotEquals(String),
    /// Field must exist (have any value).
    Exists,
    /// Field must not exist.
    #[serde(alias = "notexists")]
    NotExists,
    /// Field must be one of the specified values.
    In(Vec<String>),
    /// Field must not be any of the specified values.
    #[serde(alias = "notin")]
    NotIn(Vec<String>),
}

/// [`PlayerAllowFilter`] hold the player filter configuration (blocks all if empty).
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PlayerAllowFilter {
    /// List of player usernames to allow (disabled if empty).
    pub usernames: Option<Vec<String>>,

    /// Regex of player usernames to allow (disabled if empty).
    pub username: Option<String>,

    /// List of player IDs to allow (disabled if empty).
    pub ids: Option<Vec<String>>,
}

/// [`PlayerBlockFilter`] hold the player filter configuration (allows all if empty).
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PlayerBlockFilter {
    /// List of player usernames to block (disabled if empty).
    pub usernames: Option<Vec<String>>,

    /// Regex of player usernames to block (disabled if empty).
    pub username: Option<String>,

    /// List of player IDs to block (disabled if empty).
    pub ids: Option<Vec<String>>,
}

/// [`PlayerFillStrategy`] hold the player fill strategy configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PlayerFillStrategy {
    /// The name of the field that stores the player amount.
    pub field: String,

    /// The number of players that will be filled at maximum.
    #[serde(alias = "maxplayers")]
    pub max_players: u32,
}

/// [`GrpcDiscoveryAction`] hold the gRPC discovery action configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcDiscoveryAction {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`AuthenticationAdapter`] hold the authentication adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthenticationAdapter {
    Disabled,
    Fixed(FixedAuthentication),
    Grpc(GrpcAuthentication),
    Mojang(MojangAuthentication),
}

impl Default for AuthenticationAdapter {
    fn default() -> Self {
        Self::Mojang(MojangAuthentication::default())
    }
}

/// [`FixedAuthentication`] hold the fixed authentication configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FixedAuthentication {
    /// The fixed profile that should be used for authentication.
    pub profile: Option<Profile>,
}

/// [`GrpcAuthentication`] hold the gRPC authentication configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcAuthentication {
    /// The address of the gRPC adapter server.
    pub address: String,
}

/// [`MojangAuthentication`] hold the mojang authentication configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MojangAuthentication {
    /// The server id passed to the Mojang authentication server.
    #[serde(alias = "serverid")]
    pub server_id: String,
}

/// [`LocalizationAdapter`] hold the localization adapter configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalizationAdapter {
    Fixed(FixedLocalization),
    Grpc(GrpcLocalization),
}

impl Default for LocalizationAdapter {
    fn default() -> Self {
        Self::Fixed(FixedLocalization::default())
    }
}

/// [`FixedLocalization`] hold the fixed localization configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FixedLocalization {
    /// The locale to be used in case the client locale is unknown or unsupported.
    #[serde(alias = "defaultlocale")]
    pub default_locale: String,

    /// The localizable messages.
    pub messages: HashMap<String, HashMap<String, String>>,

    /// Whether to warn about unknown keys.
    #[serde(alias = "warnunknownkeys")]
    pub warn_unknown_keys: bool,
}

impl Default for FixedLocalization {
    fn default() -> Self {
        Self {
            default_locale: "en_US".to_string(),
            warn_unknown_keys: true,
            messages: hashmap! {
                "en" => hashmap! {
                    "locale" => "English",
                    "disconnect_timeout" => "{\"text\":\"Disconnected: No response from client (keep-alive timeout)\"}",
                    "disconnect_no_target" => "{\"text\":\"Disconnected: No available server to handle your connection\"}",
                    "disconnect_unauthenticated" => "{\"text\":\"Disconnected: Could not authenticate client\"}",
                },
                "es" => hashmap! {
                    "locale" => "Español",
                    "disconnect_timeout" => "{\"text\":\"Desconectado: No hubo respuesta del cliente (tiempo de espera agotado)\"}",
                    "disconnect_no_target" => "{\"text\":\"Desconectado: No hay un servidor disponible para manejar tu conexión\"}",
                    "disconnect_unauthenticated" => "{\"text\":\"Desconectado: No se pudo autenticar el cliente\"}",
                },
                "fr" => hashmap! {
                    "locale" => "Français",
                    "disconnect_timeout" => "{\"text\":\"Déconnecté : aucune réponse du client (délai de keep-alive dépassé)\"}",
                    "disconnect_no_target" => "{\"text\":\"Déconnecté : aucun serveur disponible pour traiter votre connexion\"}",
                    "disconnect_unauthenticated" => "{\"text\":\"DDéconnecté : Impossible d’authentifier le client\"}",
                },
                "de" => hashmap! {
                    "locale" => "Deutsch",
                    "disconnect_timeout" => "{\"text\":\"Verbindung getrennt: Keine Antwort vom Client (Keep-Alive-Timeout)\"}",
                    "disconnect_no_target" => "{\"text\":\"Verbindung getrennt: Kein verfügbarer Server für diese Verbindung\"}",
                    "disconnect_unauthenticated" => "{\"text\":\"Verbindung getrennt: Client konnte nicht authentifiziert werden\"}",
                },
                "zh-CN" => hashmap! {
                    "locale" => "简体中文",
                    "disconnect_timeout" => "{\"text\":\"已断开连接：客户端无响应（保持连接超时）\"}",
                    "disconnect_no_target" => "{\"text\":\"已断开连接：没有可用的服务器来处理你的连接\"}",
                    "disconnect_no_target" => "{\"text\":\"已断开连接：无法验证客户端\"}",
                },
                "ru" => hashmap! {
                    "locale" => "English",
                    "disconnect_timeout" => "{\"text\":\"Отключено: нет ответа от клиента (тайм-аут keep-alive)\"}",
                    "disconnect_no_target" => "{\"text\":\"Отключено: нет доступного сервера для обработки подключения\"}",
                    "disconnect_unauthenticated" => "{\"text\":\"Отключено: не удалось аутентифицировать клиента\"}",
                },
            },
        }
    }
}

/// [`GrpcLocalization`] hold the gRPC localization configuration.
#[derive(Default, Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GrpcLocalization {
    /// The address of the gRPC adapter server.
    pub address: String,
}

impl Config {
    /// Creates a new application configuration as described in the [module documentation](crate::config).
    pub fn read() -> Result<Self, ConfigError> {
        // the environment prefix for all `Config` fields
        let env_prefix = env::var("ENV_PREFIX").unwrap_or("passage".into());
        // the path of the custom configuration file
        let config_file = env::var("CONFIG_FILE").unwrap_or("config/config".into());
        let auth_secret_file = env::var("AUTH_SECRET_FILE").unwrap_or("config/auth_secret".into());

        let s = config::Config::builder()
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
