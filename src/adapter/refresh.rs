use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

#[macro_export]
macro_rules! refresh {
    ($shared:ident = $dur:expr => $fetch:expr) => {
        use tracing::{debug, warn};

        let shared = Refreshable::clone(&$shared);
        let mut interval = tokio::time::interval($dur);
        tokio::spawn(async move {
            debug!("Starting refresh task");
            loop {
                select! {
                    biased;
                    _ = shared.cancelled() => break,
                    _ = interval.tick() => {
                        match $fetch.await {
                            Ok(next) => *shared.write().await = next,
                            Err(err) => warn!(err = %err, "Failed refresh")
                        };
                    },
                }
            }
            info!("Stopped refresh task");
        });
    };
}

#[derive(Clone)]
pub struct Refreshable<T> {
    inner: Arc<RwLock<T>>,
    token: CancellationToken,
}

impl<T> Refreshable<T>
where
    T: Clone + Debug + Send + Sync + 'static,
{
    pub fn new(default: T) -> Self {
        let inner = Arc::new(RwLock::new(default));
        let token = CancellationToken::new();
        Self { inner, token }
    }

    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.token.cancelled()
    }

    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        self.inner.read().await
    }

    pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
        self.inner.write().await
    }
}

impl<T> Drop for Refreshable<T> {
    fn drop(&mut self) {
        self.token.cancel();
    }
}
