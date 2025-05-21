use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::adapter::status::Protocol;
use crate::adapter::Error;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use uuid::Uuid;

pub struct ImpackableResourcepackSupplier {
    pub reqwest_client: reqwest::Client,
    pub base_url: String,
    pub username: String,
    pub password: String,
    pub channel: String,
    pub uuid: Uuid,
    pub forced: bool,
    pub cache_duration: Duration,
}

impl ImpackableResourcepackSupplier {
    pub fn new(
        base_url: String,
        username: String,
        password: String,
        channel: String,
        uuid: Uuid,
        forced: bool,
        cache_duration: u64,
    ) -> Result<Self, Error> {
        Ok(Self {
            reqwest_client: reqwest::Client::builder().build().map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: "resourcepack",
                    cause: err.into(),
                }
            })?,
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
            channel,
            uuid,
            forced,
            cache_duration: Duration::from_secs(cache_duration),
        })
    }
}

#[async_trait]
impl ResourcepackSupplier for ImpackableResourcepackSupplier {
    async fn get_resourcepacks(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<Resourcepack>, Error> {
        // issue a request to Mojang's authentication endpoint
        let url = format!("{}/query/{}", self.base_url, self.channel);
        let response = self
            .reqwest_client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await?
            .error_for_status()?;

        let packs: Vec<ImpackableResourcepack> = response.json().await?;
        Ok(packs
            .first()
            .map(|pack| Resourcepack {
                uuid: self.uuid,
                url: format!("{}/download/{}", self.base_url, pack.id),
                hash: pack.hash.clone(),
                forced: self.forced,
                prompt_message: None,
            })
            .into_iter()
            .collect())
    }
}

#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ImpackableResourcepack {
    pub id: String,
    pub hash: String,
    pub size: u64,
}
