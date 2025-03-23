pub mod simple;

use crate::protocol::Error;
use crate::status::Protocol;
use crate::target_selector::Target;
use std::net::SocketAddr;
use uuid::Uuid;

#[trait_variant::make(TargetSelectorStrategy: Send)]
pub trait LocalTargetSelectorStrategy {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error>;
}
