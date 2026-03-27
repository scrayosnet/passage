use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result, metrics};
use regex::Regex;
use std::net::SocketAddr;
use tokio::time::Instant;
use tracing::trace;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "player_allow_filter_adapter";

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

impl FilterAdapter for PlayerAllowFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn filter(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        (username, user_id): (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
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
            if items.iter().any(|item| item == username) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(targets);
            }
        }

        // check username
        if let Some(item) = &self.username {
            trace!("filtering allow username");
            if item.is_match(username) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(targets);
            }
        }

        // check ids
        if let Some(items) = &self.ids {
            trace!("filtering allow ids");
            if items.iter().any(|item| item == user_id) {
                metrics::adapter_duration::record(ADAPTER_TYPE, start);
                return Ok(targets);
            }
        }

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(vec![])
    }
}
