pub(crate) mod fixed;
pub mod none;

use crate::connection::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

#[async_trait]
pub trait ResourcepackSupplier: Send + Sync {
    async fn get_resourcepacks(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Vec<Resourcepack>, Error>;
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Resourcepack {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    pub prompt_message: Option<String>,
}
