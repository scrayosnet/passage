use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::adapter::status::Protocol;
use crate::adapter::Error;
use async_trait::async_trait;
use reqwest::Url;
use std::net::SocketAddr;
use std::time::Duration;
use uuid::Uuid;

pub struct ImpackableResourcepackSupplier {
    pub base_uri: Url,
    pub username: String,
    pub password: String,
    pub cache_duration: Duration,
}

impl ImpackableResourcepackSupplier {
    pub fn new(
        base_uri: Url,
        username: String,
        password: String,
        cache_duration: Duration,
    ) -> Self {
        Self {
            base_uri,
            username,
            password,
            cache_duration,
        }
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
        Ok(vec![])
    }
}
