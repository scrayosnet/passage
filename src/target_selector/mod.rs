pub mod simple;

use crate::protocol::Error;
use crate::status::Protocol;
use std::iter::Map;
use std::net::SocketAddr;
use uuid::Uuid;

#[trait_variant::make(TargetSelector: Send)]
pub trait LocalTargetSelector {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        user_id: &Uuid,
        username: &str,
    ) -> Result<Option<SocketAddr>, Error>;
}

#[derive(Debug, Clone)]
pub struct Target {
    pub identifier: String,
    pub address: SocketAddr,
    pub meta: Map<String, String>,
}
