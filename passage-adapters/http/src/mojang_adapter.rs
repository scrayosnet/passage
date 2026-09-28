use crate::HTTP_CLIENT;
use passage_adapters::authentication::{AuthenticationAdapter, Profile, minecraft_hash};
use passage_adapters::{Client, Player, metrics, reject};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tracing::{debug, instrument};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "mojang_authentication_adapter";

/// The host the session server lives on, reported as `server.address`.
const SESSION_SERVER: &str = "sessionserver.mojang.com";

/// The endpoint the join is verified against, reported as `url.path`.
///
/// The full URL is deliberately not recorded: it carries the player's name and the handshake hash,
/// neither of which belongs in a span attribute.
const HAS_JOINED_PATH: &str = "/session/minecraft/hasJoined";

/// Authentication adapter that validates players against Mojang's session server.
///
/// During login, the client performs an encryption handshake that includes sending its profile to
/// the Mojang session server. This adapter verifies that handshake by contacting
/// `sessionserver.mojang.com` and returns the profile if the player has authenticated.
#[derive(Default)]
pub struct MojangAdapter {
    /// The ID of this server, it will be included in the Mojang session verification request. Can
    /// be left empty for most use-cases.
    server_id: String,
}

impl MojangAdapter {
    /// Sets the server ID included in the Mojang session verification request (builder style).
    ///
    /// The server ID is a free-form string that Minecraft sends to the session server so it can
    /// distinguish authentication requests from different servers.
    pub fn with_server_id(mut self, server_id: String) -> Self {
        self.server_id = server_id;
        self
    }

    async fn authenticate(
        &self,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        // Calculate the minecraft hash for this secret, key and username
        let hash = minecraft_hash(&self.server_id, shared_secret, encoded_public);

        // Issue a request to the Mojang authentication endpoint
        let username = &player.name;
        let url = format!(
            "https://{SESSION_SERVER}{HAS_JOINED_PATH}?username={username}&serverId={hash}"
        );
        let response = HTTP_CLIENT
            .get(&url)
            .send()
            .await
            .map_err(|err| passage_adapters::AdapterError::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: Box::new(err),
            })?
            .error_for_status()
            .map_err(|err| passage_adapters::AdapterError::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: Box::new(err),
            })?;

        // If the response is empty, then the client did not make an auth request
        if response.status() == 204 {
            debug!(
                http.response.status_code = response.status().as_u16(),
                "client did not make an authentication request",
            );
            return Err(reject(ADAPTER_TYPE));
        }

        // Parse the response profile
        let profile =
            response
                .json()
                .await
                .map_err(|err| passage_adapters::AdapterError::FailedParse {
                    adapter_type: ADAPTER_TYPE,
                    cause: Box::new(err),
                })?;
        Ok(profile)
    }
}

impl Debug for MojangAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "MojangAdapter")
    }
}

impl AuthenticationAdapter for MojangAdapter {
    #[instrument(
        level = "info",
        name = "authenticate",
        skip_all,
        fields(
            otel.kind = "client",
            http.request.method = "GET",
            server.address = SESSION_SERVER,
            url.path = HAS_JOINED_PATH,
            adapter = ADAPTER_TYPE,
            player = %player.name,
        ),
    )]
    async fn authenticate(
        &self,
        _client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        let start = Instant::now();
        let profile = self
            .authenticate(player, shared_secret, encoded_public)
            .await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        if let Err(err) = &profile {
            debug!(err = %err, "the session server did not vouch for the player");
        }
        profile
    }
}
