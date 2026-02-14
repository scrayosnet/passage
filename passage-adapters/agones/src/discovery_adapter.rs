use crate::{GameServer, META_STATE};
use futures_util::{StreamExt, TryStreamExt};
use kube::runtime::watcher::Config;
use kube::runtime::{WatchStreamExt, watcher};
use kube::{Api, Client};
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Error, Target};
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

// re-export watcher for configuration config
pub mod watcher_config {
    pub use kube::runtime::watcher::*;
}

pub struct AgonesDiscoveryAdapter {
    inner: Arc<RwLock<Vec<Target>>>,
    token: CancellationToken,
}

impl Debug for AgonesDiscoveryAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgonesDiscoveryAdapter")
    }
}

impl AgonesDiscoveryAdapter {
    pub async fn new(namespace: Option<String>, watch_config: Config) -> Result<Self, Error> {
        let inner: Arc<RwLock<Vec<Target>>> = Arc::new(RwLock::new(Vec::new()));
        let token = CancellationToken::new();

        // get stream with of game servers
        let client = Client::try_default()
            .await
            .map_err(|err| Error::FailedInitialization {
                adapter_type: "agones_target_strategy",
                cause: err.into(),
            })?;
        let servers: Api<GameServer> = if let Some(namespace) = namespace {
            Api::namespaced(client.clone(), &namespace)
        } else {
            Api::all(client.clone())
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
                let maybe_server = tokio::select! {
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
                    inner.swap_remove(found);
                }
            }
        });

        Ok(Self { inner, token })
    }
}

impl Drop for AgonesDiscoveryAdapter {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

impl DiscoveryAdapter for AgonesDiscoveryAdapter {
    async fn discover(&self) -> passage_adapters::Result<Vec<Target>> {
        Ok(self.inner.read().await.clone())
    }
}
