pub mod fixed;
#[cfg(feature = "grpc")]
pub mod grpc;
pub mod player_fill;

use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::Error;
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
    ) -> Result<Option<Target>, Error>;
}

pub trait TargetFilterExt {
    fn matches(&self, target: &Target, username: &str, user_id: &Uuid) -> bool;
}

impl TargetFilterExt for TargetFilter {
    fn matches(&self, target: &Target, username: &str, user_id: &Uuid) -> bool {
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

        // check whether the user is allowed to connect to the target
        if let Some(allow_list) = &self.allow_list {
            let has_username = allow_list.contains(&username.to_string());
            let has_user_id = allow_list.contains(&user_id.to_string());
            if !has_username && !has_user_id {
                return false;
            }
        }

        true
    }
}
