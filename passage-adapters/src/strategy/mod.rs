pub mod any;
pub mod player_fill;

use crate::{Protocol, Reason, Target, error::Result};
use std::fmt::Debug;
use std::net::SocketAddr;
use uuid::Uuid;

pub trait StrategyAdapter: Debug + Send + Sync {
    fn strategize(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> impl Future<Output = Result<Reason<Target>>> + Send;
}
