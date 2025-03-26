mod none;

use crate::protocol::Error;
use crate::status::Protocol;
use serde::Serialize;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait ResourcePackSupplier: Send {
    fn get_resource_packs(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> impl Future<Output=Result<Vec<ResourcePack>, Error>> + Send;
}

#[derive(Debug, Serialize, Clone)]
pub struct ResourcePack {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    pub prompt_message: String,
}
