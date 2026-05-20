pub mod fixed;

use crate::{Client, ServerStatus, error::Result};
use std::fmt::Debug;

/// The [`StatusAdapter`] is used to provide the server status shown to clients during a ping request.
/// If no status is returned, then the default is used.
pub trait StatusAdapter: Debug + Send + Sync {
    fn status(&self, client: &Client) -> impl Future<Output = Result<Option<ServerStatus>>> + Send;
}
