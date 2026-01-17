use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::AgonesTargetDiscovery as AgonesConfig;
use async_trait::async_trait;
use futures_util::TryStreamExt;
use futures_util::stream::StreamExt;
use kube::runtime::watcher::{Config, InitialListStrategy, ListSemantic};
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{AddrParseError, IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::select;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

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

#[derive(thiserror::Error, Debug)]
pub enum GameServerError {
    #[error("server has no identifier")]
    NoName,

    #[error("server {identifier} has no status")]
    NotStatus {
        /// The identifier of the server.
        identifier: String,
    },

    #[error("server {identifier} ip address could not be parsed: {cause}")]
    InvalidAddress {
        /// The identifier of the server.
        identifier: String,
        /// The cause of the error.
        #[source]
        cause: Box<AddrParseError>,
    },

    #[error("server is not public: {identifier}")]
    NotPublic {
        /// The identifier of the server.
        identifier: String,
    },
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

pub struct AgonesTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
    inner: Arc<RwLock<Vec<Target>>>,
    token: CancellationToken,
}

impl AgonesTargetSelector {
    pub async fn new(
        strategy: Arc<dyn TargetSelectorStrategy>,
        config: AgonesConfig,
    ) -> Result<Self, Error> {
        let inner: Arc<RwLock<Vec<Target>>> = Arc::new(RwLock::new(Vec::new()));
        let token = CancellationToken::new();

        // get stream with of game servers
        let client = Client::try_default()
            .await
            .map_err(|err| Error::FailedInitialization {
                adapter_type: "agones_target_strategy",
                cause: err.into(),
            })?;
        let servers: Api<GameServer> = Api::namespaced(client.clone(), &config.namespace);

        // create the filtering config
        let watch_config = Config {
            bookmarks: true,
            label_selector: config.label_selector,
            field_selector: None,
            timeout: None,
            list_semantic: ListSemantic::default(),
            page_size: Some(500),
            initial_list_strategy: InitialListStrategy::default(),
        };

        // create the watch stream
        let mut stream = watcher(servers, watch_config)
            .default_backoff()
            .applied_objects()
            .boxed();

        // start listener
        let _inner = Arc::clone(&inner);
        let _token = token.clone();
        tokio::spawn(async move {
            info!("starting game server watcher");
            loop {
                // get next server update
                let maybe_server = select! {
                    biased;
                    _ = _token.cancelled() => break,
                    maybe_server = stream.try_next() => maybe_server,
                };

                let server = match maybe_server {
                    Ok(Some(server)) => server,
                    Ok(None) => break,
                    Err(err) => {
                        warn!(err = ?err, "error while watching game servers");
                        continue;
                    }
                };

                // map to target
                let target: Target = match server.try_into() {
                    Ok(target) => target,
                    Err(err) => {
                        warn!(err = ?err, "error while converting game server to target");
                        continue;
                    }
                };

                // if ready, replace or push
                let mut inner = _inner.write().await;
                let state = target.meta.get(META_STATE).cloned().unwrap_or_default();
                if state == "Ready" || state == "Allocated" {
                    info!(uid = target.identifier, "adding game server to cache");
                    let found = inner.iter_mut().find(|i| i.identifier == target.identifier);
                    match found {
                        Some(found) => *found = target,
                        None => inner.push(target),
                    }
                    continue;
                }

                // remove
                info!(uid = target.identifier, "removing game server from cache");
                let found = inner.iter().position(|i| i.identifier == target.identifier);
                if let Some(found) = found {
                    inner.remove(found);
                }
            }
        });

        Ok(Self {
            strategy,
            inner,
            token,
        })
    }
}

impl Drop for AgonesTargetSelector {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

#[async_trait]
impl TargetSelector for AgonesTargetSelector {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Option<Target>, Error> {
        let inner = self.inner.read().await;
        strategize(
            Arc::clone(&self.strategy),
            client_addr,
            server_addr,
            protocol,
            username,
            user_id,
            inner.as_slice(),
        )
        .await
    }
}
