//! An adapter that does not answer until a test says so.
//!
//! Every built-in adapter resolves without ever yielding, so nothing a test drives through them can
//! observe a connection *while* an adapter call is outstanding -- which is exactly the window the
//! step machine has to hold shut.

use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::status::StatusAdapter;
use passage_adapters::{Client, Player, Result, ServerStatus, ServerVersion};
use passage_core::ProtocolVersion;
use std::sync::Arc;
use tokio::sync::Notify;

/// A status adapter that waits for [`release`](HeldStatusAdapter::release) before it answers.
#[derive(Debug, Default)]
pub struct HeldStatusAdapter {
    /// Notified when the answer may be given.
    released: Arc<Notify>,
}

impl HeldStatusAdapter {
    /// Creates the adapter, and the handle that lets it answer.
    pub fn new() -> (Self, Release) {
        let released = Arc::new(Notify::new());
        (
            Self {
                released: Arc::clone(&released),
            },
            Release(released),
        )
    }
}

/// Lets a held adapter answer.
pub struct Release(Arc<Notify>);

impl Release {
    /// Answers every call that is waiting, and the next one to arrive.
    pub fn release(&self) {
        self.0.notify_waiters();
        self.0.notify_one();
    }
}

impl StatusAdapter for HeldStatusAdapter {
    async fn status(&self, _client: &Client) -> Result<Option<ServerStatus>> {
        self.released.notified().await;
        Ok(Some(ServerStatus {
            version: ServerVersion {
                name: "held".into(),
                protocol: ProtocolVersion::UNKNOWN,
            },
            players: None,
            description: None,
            favicon: None,
            enforces_secure_chat: None,
        }))
    }
}

/// An authentication adapter that waits for [`Release::release`] before it answers.
#[derive(Debug)]
pub struct HeldAuthenticationAdapter {
    /// Notified when the answer may be given.
    released: Arc<Notify>,

    /// The profile it answers with, once it does.
    profile: Profile,
}

impl HeldAuthenticationAdapter {
    /// Creates the adapter, and the handle that lets it answer with `profile`.
    pub fn new(profile: Profile) -> (Self, Release) {
        let released = Arc::new(Notify::new());
        (
            Self {
                released: Arc::clone(&released),
                profile,
            },
            Release(released),
        )
    }
}

impl AuthenticationAdapter for HeldAuthenticationAdapter {
    async fn authenticate(
        &self,
        _client: &Client,
        _player: &Player,
        _shared_secret: &[u8],
        _encoded_public: &[u8],
    ) -> Result<Profile> {
        self.released.notified().await;
        Ok(self.profile.clone())
    }
}
