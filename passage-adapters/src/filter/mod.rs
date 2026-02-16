pub mod meta;
pub mod option;
pub mod player_allow;
pub mod player_block;

use crate::{Protocol, Target, error::Result};
use std::fmt::Debug;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait FilterAdapter: Debug + Send + Sync {
    fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> impl Future<Output = Result<Vec<Target>>> + Send;
}

impl<T> FilterAdapter for Vec<T>
where
    T: FilterAdapter,
{
    async fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
        let mut filtered_targets = targets;
        for adapter in self {
            filtered_targets = adapter
                .filter(client_addr, server_addr, protocol, user, filtered_targets)
                .await?;
        }
        Ok(filtered_targets)
    }
}
