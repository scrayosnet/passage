pub mod fixed;

use crate::{Target, error::Result};
use std::fmt::Debug;

pub trait DiscoveryAdapter: Debug + Send + Sync {
    /** Discovers all targets in the network. */
    fn discover(&self) -> impl Future<Output = Result<Vec<Target>>> + Send;
}
