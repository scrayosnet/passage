pub mod fixed;

use crate::{Client, Target, error::Result};
use std::fmt::Debug;

pub trait DiscoveryAdapter: Debug + Send + Sync {
    /** Discovers all targets in the network for the client. */
    fn discover(&self, client: &Client) -> impl Future<Output = Result<Vec<Target>>> + Send;
}
