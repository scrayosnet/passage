use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::config::MongodbStatus as MongodbConfig;
use async_trait::async_trait;
use futures_util::StreamExt;
use mongodb::bson::{Document, from_document};
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
                        match Self::refresh(&client, &config.database, &config.collection, &config.aggregation).await {
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
        database: &str,
        collection: &str,
        aggregate: &str,
    ) -> Result<Option<ServerStatus>, Error> {
        // prepare the mongo settings and query
        let mongo_db = client.database(database);
        let mongo_coll: Collection<Document> = mongo_db.collection(collection);
        let aggregate: Vec<Document> = serde_json::from_str(aggregate)?;

        // execute the aggregation pipeline and get the first result
        let mut cursor = mongo_coll.aggregate(aggregate.clone()).await?;
        let Some(document) = cursor.next().await.transpose()? else {
            return Ok(None);
        };

        // convert the document to a status
        Ok(from_document(document)?)
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
