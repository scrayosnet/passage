use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result};
use regex::Regex;
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct PlayerFilterAdapter {
    /// List of player usernames to allow (disabled if empty).
    allow_usernames: Option<Vec<String>>,

    /// Regex of player usernames to allow (disabled if empty).
    allow_username: Option<Regex>,

    /// List of player IDs to allow (disabled if empty).
    allow_ids: Option<Vec<Uuid>>,

    /// List of player usernames to block (disabled if empty).
    block_usernames: Option<Vec<String>>,

    /// Regex of player usernames to block (disabled if empty).
    block_username: Option<Regex>,

    /// List of player IDs to block (disabled if empty).
    block_ids: Option<Vec<Uuid>>,
}

impl PlayerFilterAdapter {
    pub fn new(
        allow_usernames: Option<Vec<String>>,
        allow_username: Option<Regex>,
        allow_ids: Option<Vec<Uuid>>,
        block_usernames: Option<Vec<String>>,
        block_username: Option<Regex>,
        block_ids: Option<Vec<Uuid>>,
    ) -> Self {
        Self {
            allow_usernames,
            allow_username,
            allow_ids,
            block_usernames,
            block_username,
            block_ids,
        }
    }
}

impl FilterAdapter for PlayerFilterAdapter {
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
            allow_usernames = self.allow_usernames.is_some(),
            allow_username = self.allow_username.is_some(),
            allow_ids = self.allow_ids.is_some(),
            block_usernames = self.block_usernames.is_some(),
            block_username = self.block_username.is_some(),
            block_ids = self.block_ids.is_some(),
            "filtering players"
        );

        // check usernames
        if let Some(allow_usernames) = &self.allow_usernames {
            trace!("filtering allow usernames");
            if allow_usernames.iter().any(|item| item == username) {
                return Ok(targets);
            }
            trace!("username not found");
            return Ok(vec![]);
        }

        // check username
        if let Some(allow_username) = &self.allow_username {
            trace!("filtering allow username");
            if allow_username.is_match(username) {
                return Ok(targets);
            }
            trace!("username not matching");
            return Ok(vec![]);
        }

        // check ids
        if let Some(allow_ids) = &self.allow_ids {
            trace!("filtering allow ids");
            if allow_ids.iter().any(|item| item == user_id) {
                return Ok(targets);
            }
            trace!("id not found");
            return Ok(vec![]);
        }

        // check usernames
        if let Some(block_usernames) = &self.block_usernames {
            trace!("filtering block usernames");
            if block_usernames.iter().any(|item| item == username) {
                return Ok(vec![]);
            }
            trace!("username not found");
            return Ok(targets);
        }

        // check username
        if let Some(block_username) = &self.block_username {
            trace!("filtering block username");
            if block_username.is_match(username) {
                return Ok(vec![]);
            }
            trace!("username not matching");
            return Ok(targets);
        }

        // check ids
        if let Some(block_ids) = &self.block_ids {
            trace!("filtering block ids");
            if block_ids.iter().any(|item| item == user_id) {
                return Ok(vec![]);
            }
            trace!("id not found");
            return Ok(targets);
        }

        Ok(targets)
    }
}
