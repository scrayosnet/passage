pub mod any;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod none;
pub mod player_fill;

use crate::adapter::Error;
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::config::TargetFilter;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

#[async_trait]
pub trait TargetSelectorStrategy: Send + Sync {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error>;
}

pub trait TargetFilterExt {
    fn matches(&self, target: &Target) -> bool;
}

impl TargetFilterExt for TargetFilter {
    fn matches(&self, target: &Target) -> bool {
        // check target identifier
        if let Some(identifier) = &self.identifier {
            if &target.identifier != identifier {
                return false;
            }
        }

        // check target metadata
        for (key, value) in &self.meta {
            if target.meta.get(key) == Some(value) {
                return false;
            }
        }

        true
    }
}
