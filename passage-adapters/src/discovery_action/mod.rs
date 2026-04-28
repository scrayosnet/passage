use crate::{Client, DiscoveryAdapter, Player, Target, error::Result};
use std::fmt::Debug;

pub mod full_filter;
pub mod meta_filter;
pub mod player_allow_filter;
pub mod player_block_filter;
pub mod player_fill_strategy;

pub trait DiscoveryActionAdapter: Debug + Send + Sync {
    fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> impl Future<Output = Result<()>> + Send;
}

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
