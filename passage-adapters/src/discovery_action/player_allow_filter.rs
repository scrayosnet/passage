use crate::{Client, DiscoveryActionAdapter, Player, Target, error::Result, metrics};
use regex::Regex;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "player_allow_filter_adapter";

/// Discovery action adapter that clears the target list when the player is not on the allowlist.
///
/// A player is allowed when they match at least one of the configured criteria (OR logic). If no
/// criteria are configured, no player is allowed. Checks are performed in order: exact username
/// list, username regex, UUID list.
#[derive(Debug, Default)]
pub struct PlayerAllowFilterAdapter {
    /// List of player usernames to allow (disabled if empty).
    usernames: Option<Vec<String>>,

    /// Regex of player usernames to allow (disabled if empty).
    username: Option<Regex>,

    /// List of player IDs to allow (disabled if empty).
    ids: Option<Vec<Uuid>>,
}

impl PlayerAllowFilterAdapter {
    /// Creates a new `PlayerAllowFilterAdapter`.
    ///
    /// Any criterion that is `None` is skipped. A player must satisfy at least one  criterion to be
    /// allowed through.
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

impl DiscoveryActionAdapter for PlayerAllowFilterAdapter {
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
            "allowing players"
        );
        let start = Instant::now();

        // check usernames
        if let Some(items) = &self.usernames {
            trace!("filtering allow usernames");
            if items.iter().any(|item| item == &player.name) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(());
            }
        }

        // check username
        if let Some(item) = &self.username {
            trace!("filtering allow username");
            if item.is_match(&player.name) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(());
            }
        }

        // check ids
        if let Some(items) = &self.ids {
            trace!("filtering allow ids");
            if items.iter().any(|item| item == &player.id) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(());
            }
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        targets.clear();
        Ok(())
    }
}
