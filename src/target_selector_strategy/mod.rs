use crate::protocol::Error;
use crate::status::Protocol;
use crate::target_selector::Target;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait TargetSelectorStrategy: Send {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> impl Future<Output=Result<Option<SocketAddr>, Error>> + Send;
}
