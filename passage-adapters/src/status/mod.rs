pub mod fixed;

use crate::{Protocol, ServerStatus, error::Result};
use std::fmt::Debug;
use std::net::SocketAddr;

pub trait StatusAdapter: Debug + Send + Sync {
    fn status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> impl Future<Output = Result<Option<ServerStatus>>> + Send;
}
