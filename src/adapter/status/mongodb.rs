use crate::adapter::status::{Protocol, ServerStatus, StatusSupplier};
use crate::adapter::Error;
use crate::config::MongodbStatus as MongodbConfig;
use async_trait::async_trait;
use mongodb::bson::Document;
use mongodb::options::ClientOptions;
use mongodb::{Client, Collection};
use std::net::SocketAddr;

pub struct MongodbStatusSupplier {
    collection: Collection<Document>,
    filter: Document,
    field_path: Vec<String>,
}

impl MongodbStatusSupplier {
    pub async fn new(config: MongodbConfig) -> Result<Self, Error> {
        let mut options = ClientOptions::parse(&config.address).await?;
        options.app_name = Some("passage".to_string());

        let client = Client::with_options(options)?;
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
        for (i, field) in self.field_path.iter().enumerate() {
            if i == self.field_path.len() - 1 {
                // Last iteration - get string instead of document
                let json_string = match status_doc.get_str(field) {
                    Err(_) => return Ok(None),
                    Ok(json_str) => json_str,
                };
                return Ok(Some(serde_json::from_str::<ServerStatus>(json_string)?));
            }

            match status_doc.get_document(field) {
                Err(_) => return Ok(None),
                Ok(found) => status_doc = found,
            }
        }

        Ok(None)
    }
}
