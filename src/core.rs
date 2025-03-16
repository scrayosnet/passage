use crate::protocol::Error;
use crate::status::{Protocol, ServerStatus};
use std::iter::Map;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait StatusSupplier {
    async fn get_status(
        &self,
        client_addr: SocketAddr,
        server_addr: SocketAddr,
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error>;
}

pub trait TargetDiscoverer {
    async fn discover(&self) -> Result<Vec<Target>, Error>;
}

pub trait TargetSelector {
    async fn select(
        &self,
        client_addr: SocketAddr,
        server_addr: SocketAddr,
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error>;
}

#[derive(Debug, Clone)]
pub struct Target {
    pub identifier: String,
    pub address: SocketAddr,
    pub meta: Map<String, String>,
}

struct SimpleStatusSupplier {
    status: Option<ServerStatus>,
}

impl SimpleStatusSupplier {
    fn new() -> Self {
        Self { status: None }
    }

    fn from_status(status: impl Into<ServerStatus>) -> Self {
        Self { status: Some(status.into()) }
    }
}

impl StatusSupplier for SimpleStatusSupplier {
    async fn get_status(&self, _client_addr: SocketAddr, _server_addr: SocketAddr, _protocol: Protocol) -> Result<Option<ServerStatus>, Error> {
        Ok(self.status.clone())
    }
}

struct SimpleTargetDiscoverer {
    targets: Vec<Target>,
}

impl SimpleTargetDiscoverer {
    fn new() -> Self {
        Self { targets: vec![] }
    }

    fn from_targets(targets: impl Into<Vec<Target>>) -> Self {
        Self { targets: targets.into() }
    }
}

impl TargetDiscoverer for SimpleTargetDiscoverer {
    async fn discover(&self) -> Result<Vec<Target>, Error> {
        Ok(self.targets.clone())
    }
}

struct SimpleTargetSelector {
    target: Option<SocketAddr>,
}

impl SimpleTargetSelector {
    fn new() -> Self {
        Self { target: None }
    }

    fn from_target(target: impl Into<SocketAddr>) -> Self {
        Self { target: Some(target.into()) }
    }
}

impl TargetSelector for SimpleTargetSelector {
    async fn select(&self, _client_addr: SocketAddr, _server_addr: SocketAddr, _protocol: Protocol, _username: &str, _user_id: &Uuid, _targets: &[Target]) -> Result<Option<SocketAddr>, Error> {
        Ok(self.target)
    }
}
