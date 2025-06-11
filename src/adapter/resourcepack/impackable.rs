use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::adapter::status::Protocol;
use crate::adapter::Error;
use crate::config::ImpackableResourcepack as ImpackableConfig;
use crate::config::Localization;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::select;
use tokio::sync::{oneshot, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

/// The shared http client.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

type Fetched = Option<ImpackableResourcepack>;

pub struct ImpackableResourcepackSupplier {
    inner: Arc<RwLock<Option<Fetched>>>,
    cancel: Option<oneshot::Sender<()>>,
    url: String,
    uuid: Uuid,
    forced: bool,
    localization: Arc<Localization>,
}

impl ImpackableResourcepackSupplier {
    pub fn new(config: ImpackableConfig, localization: Arc<Localization>) -> Result<Self, Error> {
        let inner = Arc::new(RwLock::new(None));
        let url = format!("{}/query/{}", config.base_url, config.channel);

        // start refresh
        let _inner = Arc::clone(&inner);
        let _url = url.clone();
        let refresh_interval = Duration::from_secs(config.cache_duration);
        let (cancel, mut canceled) = oneshot::channel();
        let mut interval = tokio::time::interval(refresh_interval);
        tokio::spawn(async move {
            info!("Starting impackable resourcepack supplier cache refresh task");
            loop {
                select! {
                    biased;
                    _ = &mut canceled => break,
                    _ = interval.tick() => {
                        match Self::refresh(&_url, &config.username, &config.password).await {
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
            url,
            uuid: config.uuid,
            forced: config.forced,
            localization,
        })
    }

    async fn refresh(
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Option<ImpackableResourcepack>, Error> {
        let response = HTTP_CLIENT
            .get(url)
            .basic_auth(username, Some(password))
            .send()
            .await?
            .error_for_status()?;
        let mut packs: Vec<ImpackableResourcepack> = response.json().await?;
        Ok(packs.pop())
    }
}

impl Drop for ImpackableResourcepackSupplier {
    fn drop(&mut self) {
        let Some(cancel) = self.cancel.take() else {
            return;
        };
        if cancel.send(()).is_err() {
            warn!("Failed to cancel cache refresh task");
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
        user_locale: &str,
    ) -> Result<Vec<Resourcepack>, Error> {
        // get fetch result
        let Some(pack) = self.inner.read().await.clone() else {
            return Err(Error::AdapterUnavailable);
        };

        // get available pack if any
        let Some(first) = pack else { return Ok(vec![]) };

        let prompt_message = self.localization.localize(
            user_locale,
            "resourcepack_impackable_prompt",
            &[("{size}", format!("{} Bytes", first.size))],
        );

        Ok(vec![Resourcepack {
            uuid: self.uuid,
            url: self.url.clone(),
            hash: first.hash,
            forced: self.forced,
            prompt_message: Some(prompt_message),
        }])
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ImpackableResourcepack {
    pub id: String,
    pub hash: String,
    pub size: u64,
}
