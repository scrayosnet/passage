use serde::Serialize;
use serde_json::value::RawValue;

/// The information on the protocol version of a server.
#[derive(Debug, Serialize)]
pub struct ServerVersion {
    /// The textual protocol version to display this version visually.
    pub name: String,
    /// The numeric protocol version (for compatibility checking).
    pub protocol: i64,
}

/// The information on a single, sampled player entry.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ServerPlayer {
    /// The visual name to display this player.
    pub name: String,
    /// The unique identifier to reference this player.
    pub id: String,
}

/// The information on the current, maximum and sampled players.
#[derive(Debug, Serialize)]
pub struct ServerPlayers {
    /// The current number of players that are online at this moment.
    pub online: u32,
    /// The maximum number of players that can join (slots).
    pub max: u32,
    /// An optional list of player information samples (version hover).
    pub sample: Option<Vec<ServerPlayer>>,
}

/// The self-reported status of a pinged server with all public metadata.
#[derive(Debug, Serialize)]
pub struct ServerStatus {
    /// The version and protocol information of the server.
    pub version: ServerVersion,
    /// The current, maximum and sampled players of the server.
    pub players: Option<ServerPlayers>,
    /// The description (MOTD) of this server.
    pub description: Option<Box<RawValue>>,
    /// The optional favicon of the server.
    pub favicon: Option<String>,
    /// Whether the server enforces the use of secure chat.
    #[serde(rename = "enforcesSecureChat")]
    pub enforces_secure_chat: Option<bool>,
}
