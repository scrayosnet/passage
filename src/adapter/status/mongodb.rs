use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::config::{MongodbStatus as MongodbConfig, MongodbStatusQuery};
use async_trait::async_trait;
use futures_util::StreamExt;
use mongodb::bson::Document;
use mongodb::options::ClientOptions;
use mongodb::{Client, Collection};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::{RwLock, oneshot};
use tracing::{info, warn};

pub struct MongodbStatusSupplier {
    inner: Arc<RwLock<Option<ServerStatus>>>,
    cancel: Option<oneshot::Sender<()>>,
}

impl MongodbStatusSupplier {
    pub async fn new(config: MongodbConfig) -> Result<Self, Error> {
        let mut options = ClientOptions::parse(&config.address).await?;
        options.app_name = Some("passage".to_string());

        let client = Client::with_options(options)?;
        let inner: Arc<RwLock<Option<ServerStatus>>> = Arc::new(RwLock::new(None));

        let _inner = Arc::clone(&inner);
        let refresh_interval = Duration::from_secs(config.cache_duration);
        let (cancel, mut canceled) = oneshot::channel();
        let mut interval = tokio::time::interval(refresh_interval);
        tokio::spawn(async move {
            info!("Starting mongodb status supplier cache refresh task");
            loop {
                select! {
                    biased;
                    _ = &mut canceled => break,
                    _ = interval.tick() => {
                        match Self::refresh(&client, &config.queries).await {
                            Ok(next) => *_inner.write().await = next,
                            Err(err) => warn!(err = ?err, "Failed to refresh status cache")
                        };
                    },
                }
            }
            info!("Stopped mongodb status supplier cache refresh task");
        });

        Ok(Self {
            inner,
            cancel: Some(cancel),
        })
    }

    async fn refresh(
        client: &Client,
        queries: &[MongodbStatusQuery],
    ) -> Result<Option<ServerStatus>, Error> {
        let mut results = Vec::with_capacity(queries.len());
        for query in queries {
            // prepare the mongo settings and query
            let db = client.database(&query.database);
            let coll: Collection<Document> = db.collection(&query.collection);
            let agg: Vec<Document> = serde_json::from_str(&query.aggregation)?;

            // execute the aggregation pipeline
            let mut cursor = coll.aggregate(agg.clone()).await?;

            // if there is a result, add it to the merge set
            if let Some(document) = cursor.next().await.transpose()? {
                results.push(document);
            }
        }

        // merge the results into a single document
        let document = results.into_iter().fold(Document::new(), |mut acc, next| {
            acc.extend(next);
            acc
        });

        // convert the document to a status
        Ok(serde_json::from_str(&serde_json::to_string(&document)?).ok())
    }
}

impl Drop for MongodbStatusSupplier {
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
impl StatusSupplier for MongodbStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(self.inner.read().await.clone())
    }
}
