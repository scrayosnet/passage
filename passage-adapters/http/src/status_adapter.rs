use crate::HTTP_CLIENT;
use passage_adapters::status::StatusAdapter;
use passage_adapters::{Client, Error, ServerStatus, metrics};
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "http_status_adapter";

/// HTTP-based status adapter that resolves targets from the HTTP server. On creation, the adapter will
/// start a background task that periodically refreshes the status. The task is automatically stopped
/// once the adapter is dropped. The status is only updated if the HTTP request is successful.
pub struct HttpStatusAdapter {
    /// The resolved status. This thread-safe container is shared between the instance and its refresh
    /// task.
    inner: Arc<RwLock<Option<ServerStatus>>>,

    /// The cancellation token used to stop the background refresh task.
    token: CancellationToken,
}

impl Debug for HttpStatusAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpStatusAdapter")
    }
}

impl HttpStatusAdapter {
    /// Creates a new `HttpStatusAdapter` that polls `address` every `cache_duration` seconds.
    ///
    /// The background refresh task starts immediately and is cancelled when the adapter is dropped.
    pub fn new(address: String, cache_duration: u64) -> Result<Self, Error> {
        let refresh_interval = Duration::from_secs(cache_duration);
        let inner = Arc::new(RwLock::new(None));
        let token = CancellationToken::new();

        let _inner = inner.clone();
        let _token = token.clone();
        let mut interval = tokio::time::interval(refresh_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::spawn(async move {
            info!("starting HTTP background task");
            loop {
                select! {
                    biased;
                    _ = _token.cancelled() => break,
                    _ = interval.tick() => {
                        debug!("refreshing targets from HTTP");
                        match Self::fetch(&address).await {
                            Ok(next) => *_inner.write().await = next,
                            Err(err) => warn!(err = %err, "Failed refresh")
                        };
                    },
                }
            }
            info!("stopped HTTP background task");
        });

        Ok(Self { inner, token })
    }

    /// Fetches the next status from HTTP. Any error status will resul in the status not getting updated.
    #[instrument(skip_all)]
    async fn fetch(url: &str) -> Result<Option<ServerStatus>, Error> {
        HTTP_CLIENT
            // send fetch request
            .get(url)
            .send()
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?
            // handle status codes
            .error_for_status()
            .map_err(|err| Error::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?
            // parse response
            .json()
            .await
            .map_err(|err| Error::FailedParse {
                adapter_type: ADAPTER_TYPE,
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
    async fn status(&self, _client: &Client) -> Result<Option<ServerStatus>, Error> {
        let start = Instant::now();
        let status = self.inner.read().await.clone();
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(status)
    }
}
