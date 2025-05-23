use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use async_trait::async_trait;
use futures_util::stream::StreamExt;
use kube::runtime::watcher::Config;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client, CustomResource};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::select;
use tokio::sync::{RwLock, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

pub const META_STATE: &str = "state";
pub const META_PLAYERS: &str = "players";

#[derive(CustomResource, Debug, Serialize, Deserialize, Default, Clone, JsonSchema)]
#[kube(group = "agones.dev", version = "v1", kind = "GameServer", namespaced)]
#[kube(status = "GameServerStatus")]
pub struct GameServerSpec {
    info: String,
    #[schemars(length(min = 3))]
    name: String,
    replicas: i32,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerStatus {
    address: String,
    state: String,
    players: Option<GameServerPlayerStatus>,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default, JsonSchema)]
pub struct GameServerPlayerStatus {
    count: i64,
    capacity: i64,
    ids: Vec<String>,
}

impl TryFrom<GameServer> for Target {
    type Error = Error;

    fn try_from(server: GameServer) -> Result<Self, Error> {
        let identifier = server.metadata.uid.ok_or(Error::AdapterUnavailable)?;
        let status = server.status.ok_or(Error::AdapterUnavailable)?;
        let address = status.address.parse()?;
        let players = status.players.map(|s| s.count).unwrap_or(-1);

        Ok(Self {
            identifier,
            address,
            meta: HashMap::from([
                (META_STATE.to_string(), status.state),
                (META_PLAYERS.to_string(), players.to_string()),
            ]),
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
        namespace: &str,
    ) -> Result<Self, Error> {
        let inner: Arc<RwLock<Vec<Target>>> = Arc::new(RwLock::new(Vec::new()));

        // get stream with of game servers
        let client = Client::try_default()
            .await
            .expect("failed to create k8s client");
        let servers: Api<GameServer> = Api::namespaced(client.clone(), namespace);
        // TODO allow for filters (using config)
        let mut stream = watcher(servers, Config::default())
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
            warn!("Failed to cancel cache watcher")
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
