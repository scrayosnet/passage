pub mod fixed;

use crate::{Client, ServerStatus, error::Result};
use std::fmt::Debug;

pub trait StatusAdapter: Debug + Send + Sync {
    fn status(&self, client: &Client) -> impl Future<Output = Result<Option<ServerStatus>>> + Send;
}
