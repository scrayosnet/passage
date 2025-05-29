pub mod fixed;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod impackable;
pub mod none;

use crate::adapter::Error;
use crate::adapter::status::Protocol;
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
        user_locale: &str,
    ) -> Result<Vec<Resourcepack>, Error>;
}

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
pub struct Resourcepack {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    pub prompt_message: Option<String>,
}
