use crate::{Client, DiscoveryAdapter, Player, Target, error::Result};
use std::fmt::Debug;

pub mod meta_filter;
pub mod player_allow_filter;
pub mod player_block_filter;
pub mod player_fill_strategy;

/// The [`DiscoveryActionAdapter`] is used to mutate/provide (Minecraft server) targets for players
/// to connect to. The discovery and discovery actions are applied as a chain. Each modifying the result
/// of the previous link. Beware; discovery actions may alter the list of targets in any way, even
/// replacing it completely.
///
/// After passing the discovery and discovery actions, the first target in the list is selected. The
/// target priority is used to pass partial ordering information between the adapters. Adapters should
/// order the targets based on their priority, with lower values indicating higher priority. The priority
/// value itself may be updated by any adapter in any way (ephemeral).
///
/// Returning [`Err`] aborts the chain and disconnects the client.
pub trait DiscoveryActionAdapter: Debug + Send + Sync {
    fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Every [`DiscoveryAdapter`] is also a [`DiscoveryActionAdapter`] that appends its discovered
/// targets to the list.
impl<T> DiscoveryActionAdapter for T
where
    T: DiscoveryAdapter,
{
    async fn apply(
        &self,
        client: &Client,
        _player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        targets.extend(self.discover(client).await?);
        Ok(())
    }
}

/// A `Vec` of adapters applies each adapter in order, short-circuiting on the first error.
impl<T> DiscoveryActionAdapter for Vec<T>
where
    T: DiscoveryActionAdapter,
{
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        for adapter in self {
            adapter.apply(client, player, targets).await?;
        }
        Ok(())
    }
}
