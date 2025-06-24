use crate::adapter::refresh::Refreshable;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::adapter::Error;
use crate::config::HttpStatus as HttpStatusConfig;
use crate::refresh;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::select;
use tracing::{debug, info, warn};

/// The shared http client.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

pub struct HttpStatusSupplier {
    inner: Refreshable<Option<ServerStatus>>,
}

impl HttpStatusSupplier {
    pub async fn new(config: HttpStatusConfig) -> Result<Self, Error> {
        let refresh_interval = Duration::from_secs(config.cache_duration);
        let inner = Refreshable::new(None);

        // start thread coupled to 'inner' to refresh it
        refresh! {
            inner = refresh_interval => Self::fetch(&config.address)
        }

        Ok(Self { inner })
    }

    async fn fetch(url: &str) -> Result<Option<ServerStatus>, Error> {
        let response = HTTP_CLIENT.get(url).send().await?.error_for_status()?;
        Ok(response.json().await?)
    }
}

#[async_trait]
impl StatusSupplier for HttpStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(self.inner.read().await.clone())
    }
}
