use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result};
use regex::Regex;
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

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

impl FilterAdapter for PlayerBlockFilterAdapter {
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
            "blocking players"
        );

        // check usernames
        if let Some(items) = &self.usernames {
            trace!("filtering block usernames");
            if items.iter().any(|item| item == username) {
                return Ok(vec![]);
            }
        }

        // check username
        if let Some(item) = &self.username {
            trace!("filtering block username");
            if item.is_match(username) {
                return Ok(vec![]);
            }
        }

        // check ids
        if let Some(items) = &self.ids {
            trace!("filtering block ids");
            if items.iter().any(|item| item == user_id) {
                return Ok(vec![]);
            }
        }

        Ok(targets)
    }
}
