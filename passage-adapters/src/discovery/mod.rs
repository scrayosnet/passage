pub mod fixed;

use crate::{Client, Target, error::Result};
use std::fmt::Debug;

/// The [`DiscoveryAdapter`] is used to provide (Minecraft server) targets for players to connect to.
/// The discovery and discovery actions are applied as a chain. Each modifying the result of the previous
/// link. Beware; discovery actions may alter the list of targets in any way, even replacing it completely.
///
/// After passing the discovery and discovery actions, the first target in the list is selected. The
/// target priority is used to pass partial ordering information between the adapters. Adapters should
/// order the targets based on their priority, with lower values indicating higher priority. The priority
/// value itself may be updated by any adapter in any way (ephemeral).
///
/// Returning [`Err`] aborts the chain and disconnects the client.
pub trait DiscoveryAdapter: Debug + Send + Sync {
    /// Discovers all targets in the network available to the given client.
    fn discover(&self, client: &Client) -> impl Future<Output = Result<Vec<Target>>> + Send;
}
