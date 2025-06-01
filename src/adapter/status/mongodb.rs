use crate::adapter::Error;
use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::config::MongodbStatus as MongodbConfig;
use async_trait::async_trait;
use mongodb::bson::{Document, from_document};
use mongodb::{Client, Collection};
use std::net::SocketAddr;

pub struct MongodbStatusSupplier {
    collection: Collection<Document>,
    filter: Document,
    field_path: Vec<String>,
}

impl MongodbStatusSupplier {
    pub async fn new(config: MongodbConfig) -> Result<Self, Error> {
        let client = Client::with_uri_str(&config.address).await?;
        let database = client.database(&config.database);
        let collection = database.collection(&config.collection);
        let filter: Document = serde_json::from_str(&config.filter)?;

        Ok(Self {
            collection,
            filter,
            field_path: config.field_path,
        })
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
        let Some(document) = self.collection.find_one(self.filter.clone()).await? else {
            return Ok(None);
        };

        // does not support dots in path
        let mut status_doc = &document;
        for field in self.field_path.iter() {
            match status_doc.get_document(field) {
                Err(_) => return Ok(None),
                Ok(found) => status_doc = found,
            }
        }

        Ok(from_document(status_doc.clone())?)
    }
}
