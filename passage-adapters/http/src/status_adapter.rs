use crate::HTTP_CLIENT;
use passage_adapters::status::StatusAdapter;
use passage_adapters::{Error, Protocol, ServerStatus};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

pub struct HttpStatusAdapter {
    inner: Arc<RwLock<Option<ServerStatus>>>,
    token: CancellationToken,
}

impl Debug for HttpStatusAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpStatusAdapter")
    }
}

impl HttpStatusAdapter {
    pub fn new(address: String, cache_duration: u64) -> Result<Self, Error> {
        let refresh_interval = Duration::from_secs(cache_duration);
        let inner = Arc::new(RwLock::new(None));
        let token = CancellationToken::new();

        let _inner = inner.clone();
        let _token = token.clone();
        let mut interval = tokio::time::interval(refresh_interval);
        tokio::spawn(async move {
            debug!("Starting refresh task");
            loop {
                select! {
                    biased;
                    _ = _token.cancelled() => break,
                    _ = interval.tick() => {
                        match Self::fetch(&address).await {
                            Ok(next) => *_inner.write().await = next,
                            Err(err) => warn!(err = %err, "Failed refresh")
                        };
                    },
                }
            }
            info!("Stopped refresh task");
        });

        Ok(Self { inner, token })
    }

    #[instrument(skip_all)]
    async fn fetch(url: &str) -> Result<Option<ServerStatus>, Error> {
        HTTP_CLIENT
            // send fetch request
            .get(url)
            .send()
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "http_status",
                cause: err.into(),
            })?
            // handle status codes
            .error_for_status()
            .map_err(|err| Error::FailedFetch {
                adapter_type: "http_status",
                cause: err.into(),
            })?
            // parse response
            .json()
            .await
            .map_err(|err| Error::FailedParse {
                adapter_type: "http_status",
                cause: err.into(),
            })
    }
}

impl Drop for HttpStatusAdapter {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

impl StatusAdapter for HttpStatusAdapter {
    async fn status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(self.inner.read().await.clone())
    }
}
