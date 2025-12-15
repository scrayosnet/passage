use crate::adapter::Error;
use crate::adapter::refresh::Refreshable;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::config::MongodbStatus as MongodbConfig;
use crate::refresh;
use async_trait::async_trait;
use futures_util::StreamExt;
use mongodb::bson::Document;
use mongodb::options::ClientOptions;
use mongodb::{Client, Collection};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::select;
use tracing::{info, instrument};

pub struct MongodbStatusSupplier {
    inner: Refreshable<Option<ServerStatus>>,
}

impl MongodbStatusSupplier {
    pub async fn new(config: MongodbConfig) -> Result<Self, Error> {
        let mut options = ClientOptions::parse(&config.address).await.map_err(|err| {
            Error::FailedInitialization {
                adapter_type: "mongodb_status",
                cause: err.into(),
            }
        })?;
        options.app_name = Some("passage".to_string());
        let client = Client::with_options(options).map_err(|err| Error::FailedInitialization {
            adapter_type: "mongodb_status",
            cause: err.into(),
        })?;

        // parse the aggregations
        let mut aggregations = Vec::with_capacity(config.queries.len());
        for query in config.queries {
            let agg: Vec<Document> = serde_json::from_str(&query.aggregation).map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: "mongodb_status",
                    cause: err.into(),
                }
            })?;
            aggregations.push((query.database, query.collection, agg));
        }

        let refresh_interval = Duration::from_secs(config.cache_duration);
        let inner = Refreshable::new(None);

        // start thread coupled to 'inner' to refresh it
        refresh! {
            inner = refresh_interval => Self::fetch(&client, &aggregations)
        }

        Ok(Self { inner })
    }

    #[instrument(skip_all)]
    async fn fetch(
        client: &Client,
        queries: &[(String, String, Vec<Document>)],
    ) -> Result<Option<ServerStatus>, Error> {
        let mut results = Vec::with_capacity(queries.len());
        for query in queries {
            // prepare the mongo settings and query
            let db = client.database(&query.0);
            let coll: Collection<Document> = db.collection(&query.1);
            let agg: &[Document] = &query.2;

            // execute the aggregation pipeline
            let mut cursor =
                coll.aggregate(agg.iter().cloned())
                    .await
                    .map_err(|err| Error::FailedFetch {
                        adapter_type: "mongodb_status",
                        cause: err.into(),
                    })?;

            // if there is a result, add it to the merge set
            while let Some(maybe_document) = cursor.next().await {
                let document = maybe_document.map_err(|err| Error::FailedFetch {
                    adapter_type: "mongodb_status",
                    cause: err.into(),
                })?;
                results.push(document);
            }
        }

        // if no documents found, respond with no status
        if results.is_empty() {
            return Ok(None);
        }

        // merge the partial results into a single document
        let document = results.into_iter().fold(Document::new(), |mut acc, next| {
            acc.extend(next);
            acc
        });

        // convert the document to a status
        let status_str = serde_json::to_string(&document).map_err(|err| Error::FailedParse {
            adapter_type: "mongodb_status",
            cause: err.into(),
        })?;
        let status = serde_json::from_str(&status_str).map_err(|err| Error::FailedParse {
            adapter_type: "mongodb_status",
            cause: err.into(),
        })?;
        Ok(Some(status))
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
