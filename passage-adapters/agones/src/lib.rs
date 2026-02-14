use crate::error::GameServerError;
use kube::{CustomResource, ResourceExt};
use passage_adapters::Target;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

pub mod discovery_adapter;
pub mod error;

// reexport errors types
#[allow(unused_imports)]
pub use error::*;

// reexport adapters
pub use discovery_adapter::*;

pub const META_STATE: &str = "state";

#[derive(CustomResource, Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[kube(group = "agones.dev", version = "v1", kind = "GameServer", namespaced)]
#[kube(status = "GameServerStatus")]
pub struct GameServerSpec {
    #[serde(flatten)]
    pub additional_fields: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerStatus {
    address: String,
    #[serde(default)]
    ports: Vec<GameServerPort>,
    state: String,
    counters: Option<HashMap<String, GameServerCounter>>,
    lists: Option<HashMap<String, GameServerList>>,
    #[serde(flatten)]
    pub additional_fields: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerPort {
    pub name: String,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerCounter {
    count: Option<u32>,
    capacity: Option<u32>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerList {
    capacity: Option<u32>,
    #[serde(default)]
    values: Vec<String>,
}

impl TryFrom<GameServer> for Target {
    type Error = GameServerError;

    fn try_from(server: GameServer) -> Result<Self, GameServerError> {
        let identifier = server
            .metadata
            .name
            .clone()
            .ok_or(GameServerError::NoName)?;
        let status = server.status.clone().ok_or(GameServerError::NotStatus {
            identifier: identifier.clone(),
        })?;
        let ip: IpAddr = status
            .address
            .parse()
            .map_err(|err| GameServerError::InvalidAddress {
                identifier: identifier.clone(),
                cause: Box::new(err),
            })?;
        let port = status
            .ports
            .first()
            .map(|p| p.port)
            .ok_or(GameServerError::NotPublic {
                identifier: identifier.clone(),
            })?;
        let address = SocketAddr::new(ip, port);

        // add meta data
        let mut meta = HashMap::from([(META_STATE.to_string(), status.state)]);

        // add counters and lists
        if let Some(counters) = &status.counters {
            for (name, counter) in counters {
                meta.insert(name.clone(), counter.count.unwrap_or(0).to_string());
            }
        }
        if let Some(lists) = &status.lists {
            for (name, list) in lists {
                meta.insert(name.clone(), list.values.join(","));
            }
        }

        // add labels and annotations
        for (label, value) in server.labels() {
            meta.insert(label.clone(), value.clone());
        }
        for (annot, value) in server.annotations() {
            meta.insert(annot.clone(), value.clone());
        }

        Ok(Self {
            identifier,
            address,
            meta,
        })
    }
}
