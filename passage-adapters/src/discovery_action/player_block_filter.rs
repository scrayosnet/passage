use crate::{Client, DiscoveryActionAdapter, Player, Target, error::Result, metrics};
use regex::Regex;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "player_block_filter_adapter";

#[derive(Debug, Default)]
pub struct PlayerBlockFilterAdapter {
    /// List of player usernames to block (disabled if empty).
    usernames: Option<Vec<String>>,

    /// Regex of player usernames to block (disabled if empty).
    username: Option<Regex>,

    /// List of player IDs to block (disabled if empty).
    ids: Option<Vec<Uuid>>,
}

impl PlayerBlockFilterAdapter {
    pub fn new(
        usernames: Option<Vec<String>>,
        username: Option<Regex>,
        ids: Option<Vec<Uuid>>,
    ) -> Self {
        Self {
            usernames,
            username,
            ids,
        }
    }
}

impl DiscoveryActionAdapter for PlayerBlockFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn apply(
        &self,
        _client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        trace!(
            usernames = self.usernames.is_some(),
            username = self.username.is_some(),
            ids = self.ids.is_some(),
            "blocking players"
        );
        let start = Instant::now();

        // check usernames
        if let Some(items) = &self.usernames {
            trace!("filtering block usernames");
            if items.iter().any(|item| item == &player.name) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                targets.clear();
                return Ok(());
            }
        }

        // check username
        if let Some(item) = &self.username {
            trace!("filtering block username");
            if item.is_match(&player.name) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                targets.clear();
                return Ok(());
            }
        }

        // check ids
        if let Some(items) = &self.ids {
            trace!("filtering block ids");
            if items.iter().any(|item| item == &player.id) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                targets.clear();
                return Ok(());
            }
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(())
    }
}
