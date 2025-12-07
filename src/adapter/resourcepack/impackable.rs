use crate::adapter::Error;
use crate::adapter::refresh::Refreshable;
use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier, format_size};
use crate::adapter::status::Protocol;
use crate::config::ImpackableResourcepack as ImpackableConfig;
use crate::config::Localization;
use crate::refresh;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::select;
use tracing::{info, warn};
use uuid::Uuid;

/// The shared http client.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

pub struct ImpackableResourcepackSupplier {
    inner: Refreshable<Option<Option<ImpackableResourcepack>>>,
    base_url: String,
    uuid: Uuid,
    forced: bool,
    localization: Arc<Localization>,
}

impl ImpackableResourcepackSupplier {
    pub fn new(config: ImpackableConfig, localization: Arc<Localization>) -> Result<Self, Error> {
        let query_url = format!("{}/query/{}", config.base_url, config.channel);
        let refresh_interval = Duration::from_secs(config.cache_duration);
        let inner = Refreshable::new(None);

        // start thread coupled to 'inner' to refresh it
        refresh! {
            inner = refresh_interval => Self::fetch(&query_url, &config.username, &config.password)
        }

        Ok(Self {
            inner,
            base_url: config.base_url,
            uuid: config.uuid,
            forced: config.forced,
            localization,
        })
    }

    async fn fetch(
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<Option<Option<ImpackableResourcepack>>, Error> {
        let mut packs: Vec<ImpackableResourcepack> = HTTP_CLIENT
            // send fetch request
            .get(url)
            .basic_auth(username, Some(password))
            .send()
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "impackable_resourcepack",
                cause: err.into(),
            })?
            // handle status codes
            .error_for_status()
            .map_err(|err| Error::FailedFetch {
                adapter_type: "impackable_resourcepack",
                cause: err.into(),
            })?
            // parse response
            .json()
            .await
            .map_err(|err| Error::FailedParse {
                adapter_type: "impackable_resourcepack",
                cause: err.into(),
            })?;
        Ok(Some(packs.pop()))
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
            return Err(Error::AdapterUnavailable {
                adapter_type: "impackable_resourcepack",
                reason: "resource packs have not been initialized yet",
            });
        };

        // get available pack if any
        let Some(first) = pack else { return Ok(vec![]) };

        let prompt_message = self.localization.localize(
            user_locale,
            "resourcepack_impackable_prompt",
            &[("{size}", format_size(first.size))],
        );

        Ok(vec![Resourcepack {
            uuid: self.uuid,
            url: format!("{}/download/{}", self.base_url, first.id),
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
