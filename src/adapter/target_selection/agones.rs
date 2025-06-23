use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::AgonesTargetDiscovery as AgonesConfig;
use async_trait::async_trait;
use futures_util::stream::StreamExt;
use kube::runtime::watcher::{Config, InitialListStrategy, ListSemantic};
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, CustomResource, ResourceExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::select;
use tokio::sync::{RwLock, oneshot};
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
    values: Vec<String>,
}

impl TryFrom<GameServer> for Target {
    type Error = Error;

    fn try_from(server: GameServer) -> Result<Self, Error> {
        let identifier = server
            .metadata
            .name
            .clone()
            .ok_or(Error::AdapterUnavailable)?;
        let status = server.status.clone().ok_or(Error::AdapterUnavailable)?;

        // Parse IP address
        let ip: IpAddr = status.address.parse()?;
        // Extract port from the typed ports field
        let port = status
            .ports
            .first()
            .map(|p| p.port)
            .ok_or(Error::ServerNotPublic {
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
    cancel: Option<oneshot::Sender<()>>,
}

impl AgonesTargetSelector {
    pub async fn new(
        strategy: Arc<dyn TargetSelectorStrategy>,
        config: AgonesConfig,
    ) -> Result<Self, Error> {
        let inner: Arc<RwLock<Vec<Target>>> = Arc::new(RwLock::new(Vec::new()));

        // get stream with of game servers
        let client = Client::try_default()
            .await
            .map_err(|err| Error::FailedInitialization {
                adapter_type: "target_strategy",
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
        let (cancel, mut canceled) = oneshot::channel();
        tokio::spawn(async move {
            info!("starting game server watcher");
            loop {
                // get next server update
                let maybe_server = select! {
                    biased;
                    _ = &mut canceled => break,
                    maybe_server = stream.next() => maybe_server.transpose(),
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
            cancel: Some(cancel),
        })
    }
}

impl Drop for AgonesTargetSelector {
    fn drop(&mut self) {
        let Some(cancel) = self.cancel.take() else {
            return;
        };
        if cancel.send(()).is_err() {
            warn!("Failed to cancel cache watcher");
        }
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
    ) -> Result<Option<SocketAddr>, Error> {
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
