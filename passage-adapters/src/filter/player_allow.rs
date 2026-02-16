use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result};
use regex::Regex;
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

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

        // check usernames
        if let Some(items) = &self.usernames {
            trace!("filtering allow usernames");
            if items.iter().any(|item| item == username) {
                return Ok(targets);
            }
        }

        // check username
        if let Some(item) = &self.username {
            trace!("filtering allow username");
            if item.is_match(username) {
                return Ok(targets);
            }
        }

        // check ids
        if let Some(items) = &self.ids {
            trace!("filtering allow ids");
            if items.iter().any(|item| item == user_id) {
                return Ok(targets);
            }
        }

        Ok(vec![])
    }
}
