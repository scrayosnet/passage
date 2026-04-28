//! This module contains the adapter logic and the individual implementations of the adapters with
//! different responsibilities.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use std::collections::HashMap;
use std::net::SocketAddr;

pub mod adapters;
pub mod authentication;
pub mod discovery;
pub mod discovery_action;
pub mod error;
pub mod localization;
pub mod metrics;
pub mod status;

// reexport errors types
pub use error::*;

// reexport adapter types
pub use authentication::AuthenticationAdapter;
pub use discovery::DiscoveryAdapter;
pub use discovery_action::DiscoveryActionAdapter;
pub use localization::LocalizationAdapter;
pub use status::StatusAdapter;

// reexport adapters
pub use adapters::Adapters;
pub use authentication::disabled::DisabledAuthenticationAdapter;
pub use authentication::fixed::FixedAuthenticationAdapter;
pub use discovery::fixed::FixedDiscoveryAdapter;
pub use discovery_action::meta_filter::MetaFilterAdapter;
pub use discovery_action::player_allow_filter::PlayerAllowFilterAdapter;
pub use discovery_action::player_block_filter::PlayerBlockFilterAdapter;
pub use discovery_action::player_fill_strategy::PlayerFillStrategyAdapter;
pub use localization::fixed::FixedLocalizationAdapter;
pub use status::fixed::FixedStatusAdapter;

/// The Minecraft protocol version type.
pub type Protocol = i32;

/// Contains the client information sent to the server on the initial handshake.
#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    /// The pretended protocol version.
    pub protocol_version: Protocol,

    /// The pretended server address.
    pub server_address: String,

    /// The pretended server port.
    pub server_port: u16,

    /// The address by which the client connected.
    pub address: SocketAddr,
}

/// Contains the player information.
#[derive(Debug, Clone, Deserialize)]
pub struct Player {
    /// The name of the player.
    pub name: String,

    /// The id of the player.
    pub id: uuid::Uuid,
}

/// A target gameserver that can be connected to.
#[derive(Debug, Clone, Deserialize)]
pub struct Target {
    /// The target's unique identifier.
    pub identifier: String,

    /// The target's address.
    pub address: SocketAddr,

    /// The target's priority, lower values are preferred.
    pub priority: u16,

    /// Any metadata attached to the target that may be used by the adapters.
    #[serde(default)]
    pub meta: HashMap<String, String>,
}

/// The information on the protocol version of a server.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct ServerVersion {
    /// The textual protocol version to display this version visually.
    #[serde(default = "default_name")]
    pub name: String,
    /// The numeric protocol version (for compatibility checking).
    pub protocol: Protocol,
}

/// The information on a single, sampled player entry.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ServerPlayer {
    /// The visual name to display this player.
    pub name: String,
    /// The unique identifier to reference this player.
    pub id: String,
}

/// The information on the current, maximum and sampled players.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerPlayers {
    /// The current number of players that are online at this moment.
    pub online: u32,
    /// The maximum number of players that can join (slots).
    pub max: u32,
    /// An optional list of player information samples (version hover).
    pub sample: Option<Vec<ServerPlayer>>,
}

/// The self-reported status of a pinged server with all public metadata.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ServerStatus {
    /// The version and protocol information of the server.
    pub version: ServerVersion,
    /// The current, maximum and sampled players of the server.
    pub players: Option<ServerPlayers>,
    /// The description (MOTD) of this server.
    #[serde(deserialize_with = "deserialize_description")]
    pub description: Option<Box<RawValue>>,
    /// The optional favicon of the server.
    pub favicon: Option<String>,
    /// Whether the server enforces the use of secure chat.
    #[serde(rename = "enforcesSecureChat")]
    pub enforces_secure_chat: Option<bool>,
}

fn default_name() -> String {
    "Passage".to_owned()
}

fn deserialize_description<'de, D>(deserializer: D) -> Result<Option<Box<RawValue>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    if let Some(value) = opt {
        Ok(Some(
            RawValue::from_string(value).map_err(serde::de::Error::custom)?,
        ))
    } else {
        Ok(None)
    }
}
