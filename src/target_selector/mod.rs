pub mod first;
pub mod fixed;

use crate::protocol::Error;
use crate::status::Protocol;
use std::iter::Map;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait TargetSelector: Send {
    fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        user_id: &Uuid,
        username: &str,
    ) -> impl Future<Output=Result<Option<SocketAddr>, Error>> + Send;
}

#[derive(Debug, Clone)]
pub struct Target {
    pub identifier: String,
    pub address: SocketAddr,
    pub meta: Map<String, String>,
}
