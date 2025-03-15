use crate::protocol::Error;
use crate::status::ServerStatus;
use std::iter::Map;
use std::net::SocketAddr;

pub trait StatusSupplier {
    async fn get_status(
        // client address
        // server address
        // protocol version
    ) -> Result<Option<ServerStatus>, Error>;
}

pub trait TargetDiscoverer {
    async fn discover() -> Result<Vec<Target>, Error>;
}

pub trait TargetSelector {
    async fn select(
        // client address
        // server address
        // protocol version
        // username
        // user id
        // targets
    ) -> Result<Option<SocketAddr>, Error>;
}

pub struct Target {
    identifier: String,
    address: SocketAddr,
    meta: Map<String, String>,
}
