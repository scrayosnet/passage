pub mod fixed;
pub mod grpc;
pub mod none;

use crate::adapter::Error;
use async_trait::async_trait;
use packets::VarInt;
use serde::Serialize;
use serde_json::value::RawValue;
use std::net::SocketAddr;

pub type Protocol = VarInt;

/// The information on the protocol version of a server.
#[derive(Debug, Serialize, Clone)]
pub struct ServerVersion {
    /// The textual protocol version to display this version visually.
    pub name: String,
    /// The numeric protocol version (for compatibility checking).
    pub protocol: Protocol,
}

impl Default for ServerVersion {
    fn default() -> Self {
        Self {
            name: "JustChunks".to_owned(),
            protocol: 0,
        }
    }
}

/// The information on a single, sampled player entry.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ServerPlayer {
    /// The visual name to display this player.
    pub name: String,
    /// The unique identifier to reference this player.
    pub id: String,
}

/// The information on the current, maximum and sampled players.
#[derive(Debug, Serialize, Clone)]
pub struct ServerPlayers {
    /// The current number of players that are online at this moment.
    pub online: u32,
    /// The maximum number of players that can join (slots).
    pub max: u32,
    /// An optional list of player information samples (version hover).
    pub sample: Option<Vec<ServerPlayer>>,
}

/// The self-reported status of a pinged server with all public metadata.
#[derive(Debug, Serialize, Clone, Default)]
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

#[async_trait]
pub trait StatusSupplier: Send + Sync {
    async fn get_status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error>;
}
