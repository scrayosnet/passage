use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::config::HttpStatus as HttpStatusConfig;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::select;
use tokio::sync::{RwLock, oneshot};
use tracing::{info, warn};

/// The shared http client.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

pub struct HttpStatusSupplier {
    inner: Arc<RwLock<Option<ServerStatus>>>,
    cancel: Option<oneshot::Sender<()>>,
}

impl HttpStatusSupplier {
    pub async fn new(config: HttpStatusConfig) -> Result<Self, Error> {
        let inner: Arc<RwLock<Option<ServerStatus>>> = Arc::new(RwLock::new(None));

        let _inner = Arc::clone(&inner);
        let refresh_interval = Duration::from_secs(config.cache_duration);
        let (cancel, mut canceled) = oneshot::channel();
        let mut interval = tokio::time::interval(refresh_interval);
        tokio::spawn(async move {
            info!("Starting http status supplier cache refresh task");
            loop {
                select! {
                    biased;
                    _ = &mut canceled => break,
                    _ = interval.tick() => {
                        match Self::refresh(&config.address).await {
                            Ok(next) => *_inner.write().await = next,
                            Err(err) => warn!(err = ?err, "Failed to refresh status cache")
                        };
                    },
                }
            }
            info!("Stopped http status supplier cache refresh task");
        });

        Ok(Self {
            inner,
            cancel: Some(cancel),
        })
    }

    async fn refresh(url: &str) -> Result<Option<ServerStatus>, Error> {
        let response = HTTP_CLIENT.get(url).send().await?.error_for_status()?;
        Ok(response.json().await?)
    }
}

impl Drop for HttpStatusSupplier {
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
