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

pub fn format_size(size: u64) -> String {
    match size {
        s if s < 1 << 10 => format!("{} Bytes", s),
        s if s < 1 << 20 => format!("{:.2} KiB", s as f64 / (1 << 10) as f64),
        s if s < 1 << 30 => format!("{:.2} MiB", s as f64 / (1 << 20) as f64),
        s if s < 1 << 40 => format!("{:.2} GiB", s as f64 / (1 << 30) as f64),
        s if s < 1 << 50 => format!("{:.2} TiB", s as f64 / (1 << 40) as f64),
        s if s < 1 << 60 => format!("{:.2} PiB", s as f64 / (1 << 50) as f64),
        s => format!("{:.2} EiB", s as f64 / (1 << 60) as f64),
    }
}
