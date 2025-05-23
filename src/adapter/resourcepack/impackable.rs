use crate::adapter::Error;
use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::adapter::status::Protocol;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::select;
use tokio::sync::{RwLock, oneshot};
use tracing::{info, warn};
use uuid::Uuid;

/// The shared http client.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

pub struct ImpackableResourcepackSupplier {
    inner: Arc<RwLock<Option<Vec<Resourcepack>>>>,
    cancel: Option<oneshot::Sender<()>>,
}

impl ImpackableResourcepackSupplier {
    pub fn new(
        base_url: String,
        username: String,
        password: String,
        channel: String,
        uuid: Uuid,
        forced: bool,
        cache_duration: u64,
    ) -> Result<Self, Error> {
        let inner = Arc::new(RwLock::new(None));

        // start refresh
        let _inner = Arc::clone(&inner);
        let refresh_interval = Duration::from_secs(cache_duration);
        let (cancel, mut canceled) = oneshot::channel();
        let mut interval = tokio::time::interval(refresh_interval);
        tokio::spawn(async move {
            info!("Starting impackable resourcepack supplier cache refresh task");
            loop {
                select! {
                    biased;
                    _ = &mut canceled => break,
                    _ = interval.tick() => {
                        let result = ImpackableResourcepackSupplier::refresh(&base_url,&channel,&username,&password,&uuid,forced).await;
                        match result {
                            Ok(next) => *_inner.write().await = Some(next),
                            Err(err) => warn!(err = ?err, "Failed to refresh resourcepack cache")
                        };
                    },
                }
            }
            info!("Stopped impackable resourcepack supplier cache refresh task");
        });

        Ok(Self {
            inner,
            cancel: Some(cancel),
        })
    }

    async fn refresh(
        base_url: &str,
        channel: &str,
        username: &str,
        password: &str,
        uuid: &Uuid,
        forced: bool,
    ) -> Result<Vec<Resourcepack>, Error> {
        let url = format!("{}/query/{}", base_url, channel);

        // issue a request to Mojang's authentication endpoint
        let response = HTTP_CLIENT
            .get(&url)
            .basic_auth(username, Some(password))
            .send()
            .await?
            .error_for_status()?;

        let packs: Vec<ImpackableResourcepack> = response.json().await?;
        Ok(packs
            .first()
            .map(|pack| Resourcepack {
                uuid: *uuid,
                url,
                hash: pack.hash.clone(),
                forced,
                // TODO add support for custom messages with i18n
                prompt_message: None,
            })
            .into_iter()
            .collect())
    }
}

impl Drop for ImpackableResourcepackSupplier {
    fn drop(&mut self) {
        let Some(cancel) = self.cancel.take() else {
            return;
        };
        if cancel.send(()).is_err() {
            warn!("Failed to cancel cache refresh task")
        }
    }
}

#[async_trait]
impl ResourcepackSupplier for ImpackableResourcepackSupplier {
    async fn get_resourcepacks(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<Resourcepack>, Error> {
        self.inner
            .read()
            .await
            .clone()
            .ok_or(Error::AdapterUnavailable)
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ImpackableResourcepack {
    pub id: String,
    pub hash: String,
    pub size: u64,
}
