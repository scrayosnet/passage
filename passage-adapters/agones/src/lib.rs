use crate::error::GameServerError;
use kube::{CustomResource, ResourceExt};
use passage_adapters::Target;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

pub mod discovery_adapter;
pub mod error;
pub mod template;

// reexport errors types
#[allow(unused_imports)]
pub use error::*;

// reexport adapters
pub use discovery_adapter::*;

pub const META_STATE: &str = "state";

/// A GameServerAllocation is used to atomically allocate a `GameServer` out of a set of `GameServers`.
/// This could be a single `Fleet`, multiple `Fleets`, or a self-managed group of `GameServers`.
///
/// Allocation is the process of selecting the optimal `GameServer` that matches the filters defined in
/// the `GameServerAllocation` specification below and returning its details.
///
/// A successful allocation moves the `GameServer` to the `Allocated` state, which indicates that it
/// is currently active, likely with players on it, and should not be removed until `SDK.Shutdown()`
/// is called, or it is explicitly manually deleted.
#[derive(CustomResource, Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[kube(
    group = "allocation.agones.dev",
    version = "v1",
    kind = "GameServerAllocation",
    namespaced
)]
#[kube(status = "GameServerAllocationStatus")]
#[serde(rename_all = "camelCase")]
pub struct GameServerAllocationSpec {
    /// GameServer selector from which to choose GameServers from. Defaults to all GameServers.
    /// 'matchLabels', 'matchExpressions', 'gameServerState' and player filters can be used for filtering.
    /// See: https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/ for more details
    /// on label selectors. An ordered list of GameServer label selectors. If the first selector is
    /// not matched, the selection attempts the second selector, and so on. This is useful for things
    /// like smoke testing of new game servers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selectors: Option<Vec<serde_json::Value>>,

    /// Priorities configuration alters the order in which GameServers are searched for matches to the
    /// configured selectors. Priority of sorting is in descending importance. I.e. The position 0
    /// priority entry is checked first. For Packed strategy sorting, this priority list will be the
    /// tie-breaker within the least utilised infrastructure, to ensure optimal infrastructure usage
    /// while also allowing some custom prioritisation of GameServers. For Distributed strategy sorting,
    /// the entire selection of GameServers will be sorted by this priority list to provide the order
    /// that GameServers will be allocated by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priorities: Option<Vec<serde_json::Value>>,

    /// Defines how GameServers are organized across the cluster. Options include:
    /// - "Packed" (default) is aimed at dynamic Kubernetes clusters, such as cloud providers, wherein
    ///   we want to bin pack resources.
    /// - "Distributed" is aimed at static Kubernetes clusters, wherein we want to distribute resources
    ///   across the entire cluster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduling: Option<String>,

    /// Optional custom metadata that is added to the game server at allocation. You can use this to
    /// tell the server necessary session data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerAllocationStatus {
    /// State is the current state of a GameServerAllocation, e.g., Allocated, or UnAllocated
    pub state: Option<String>,

    /// is the name of the game server attached to this allocation, once the state is Allocated.
    pub game_server_name: Option<String>,

    /// Ports is a list of the ports that the game server makes available.
    pub ports: Vec<GameServerPort>,

    /// Address is the primary network address where the game server can be reached.
    pub address: Option<String>,

    /// NodeName is the name of the node that the gameserver is running on.
    pub node_name: Option<String>,

    /// Source is “local” unless this allocation is from a remote cluster, in which case Source is
    /// the endpoint of the remote agones-allocator.
    pub source: Option<String>,

    /// Counters (Beta, “CountsAndLists” feature flag) is a map of CounterStatus of the game server
    /// at allocation time.
    pub counters: Option<HashMap<String, GameServerCounter>>,

    /// Lists (Beta, “CountsAndLists” feature flag) is a map of ListStatus of the game server at
    /// allocation time.
    pub lists: Option<HashMap<String, GameServerList>>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerMetadata {
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerPort {
    pub name: Option<String>,
    pub port: u16,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerCounter {
    count: Option<u32>,
    capacity: Option<u32>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GameServerList {
    capacity: Option<u32>,
    #[serde(default)]
    values: Vec<String>,
}

impl TryFrom<GameServerAllocation> for Target {
    type Error = GameServerError;

    fn try_from(server: GameServerAllocation) -> Result<Self, GameServerError> {
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
            .ok_or(GameServerError::NoAddress)?
            .parse()
            .map_err(|err| GameServerError::InvalidAddress {
                identifier: identifier.clone(),
                cause: Box::new(err),
            })?;
        // TODO use configured port!
        let port = status
            .ports
            .first()
            .map(|p| p.port)
            .ok_or(GameServerError::NotPublic {
                identifier: identifier.clone(),
            })?;
        let address = SocketAddr::new(ip, port);

        // add meta data
        let mut meta = HashMap::new();

        // add state
        if let Some(state) = status.state {
            meta.insert(META_STATE.to_string(), state);
        }

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
            priority: 0,
            meta,
        })
    }
}
