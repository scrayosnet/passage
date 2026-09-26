use crate::cookie::Cookie;
use crate::crypto;
use crate::router::{DynRoute, State};
use anyhow::anyhow;
use passage_adapters::authentication::Profile;
use passage_adapters::{
    AdapterError, AuthenticationAdapter, Client, LocalizationAdapter, Player, ServerStatus,
    StatusAdapter, Target,
};
use passage_core::Phase;
use passage_core::connection::{ConnRef, DispatchError};
use passage_core::packet::{configuration, login};
use passage_core::wire::ByteString;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{debug, warn};

/// [`ConnRefExt`] provides a set of convenience methods for interacting with a connection of [`State`].
pub(crate) trait ConnRefExt {
    /// The route selected by the connection. Returns `None` if no route is selected.
    fn route(&self) -> Option<Arc<DynRoute>>;

    /// Gets the client information stored in the connection. Before the handshake packet, it only
    /// contains the [`Client::address`] (any other field is the default).
    fn client(&self) -> Client;

    /// Gets the player information stored in the connection. The data is updated iteratively, being
    /// the default at the start, getting filled, and validated.
    fn player(&self) -> Player;

    /// Gets the current locale of the connection. Returns `None` if the locale is not set.
    fn locale(&self) -> Option<String>;

    /// Queries the status adapter based on the selected route. The connection state is not updated.
    /// Any adapter errors are passed on. Returns a fetch error if no route is selected.
    fn status(&self) -> impl Future<Output = crate::Result<Option<ServerStatus>, AdapterError>>;

    /// Authorizes the current connection information using the authentication adapter. The connection
    /// state is not updated. Any adapter errors are passed on. Returns a fetch error if no route is
    /// selected.
    fn authorize(
        &self,
        shared_secret: &[u8],
    ) -> impl Future<Output = crate::Result<Profile, AdapterError>>;

    /// Localizes a key using the localization adapter. The connection state is not updated. Any
    /// adapter errors are passed on. Returns a fetch error if no route is selected.
    fn localize(
        &self,
        key: &str,
        params: &[(&'static str, String)],
    ) -> impl Future<Output = ByteString>;

    /// Selects a target using the target adapter. The connection state is not updated. Any adapter
    /// errors are passed on. Returns a fetch error if no route is selected.
    fn target(&self) -> impl Future<Output = crate::Result<Target, AdapterError>>;

    /// Queries the client for a cookie and waits for the cookie channel to be filled by any cookie
    /// handler. Supports both login and configuration phase. The connection is temporarily set to
    /// accept additional packets until the cookie is received (resetting to previous state).
    fn cookie<C: Cookie>(&self) -> impl Future<Output = crate::Result<Option<C>, DispatchError>>;
}

impl ConnRefExt for ConnRef<'_, State> {
    fn route(&self) -> Option<Arc<DynRoute>> {
        self.with(|c| c.state.route())
    }

    fn client(&self) -> Client {
        self.with(|c| c.state.client.clone())
    }

    fn player(&self) -> Player {
        self.with(|c| c.state.player.clone())
    }

    fn locale(&self) -> Option<String> {
        self.with(|c| c.state.locale.clone())
    }

    async fn status(&self) -> crate::Result<Option<ServerStatus>, AdapterError> {
        let status = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?
            .status_adapter
            .status(&self.client())
            .await?;
        Ok(status)
    }

    async fn authorize(&self, shared_secret: &[u8]) -> crate::Result<Profile, AdapterError> {
        let profile = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?
            .authentication_adapter
            .authenticate(
                &self.client(),
                &self.player(),
                shared_secret,
                &crypto::ENCODED_PUB,
            )
            .await?;
        Ok(profile)
    }

    async fn localize(&self, key: &str, params: &[(&'static str, String)]) -> ByteString {
        let Some(route) = self.route() else {
            return ByteString::from(key);
        };

        let message = route
            .localization_adapter
            .localize(self.locale().as_deref(), key, params)
            .await;
        match message {
            Ok(message) => message.try_into().expect("infallible"),
            Err(err) if err.is_rejected() => {
                debug!(err = %err, "rejected to localize key");
                ByteString::from(key)
            }
            Err(err) => {
                warn!(err = %err, "failed to localize key");
                ByteString::from(key)
            }
        }
    }

    async fn target(&self) -> crate::Result<Target, AdapterError> {
        let target = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?
            .select(&self.client(), &self.player())
            .await?;
        Ok(target)
    }

    async fn cookie<C: Cookie>(&self) -> crate::Result<Option<C>, DispatchError> {
        let (rx, secret, gated) = self.with(|c| {
            // Ensure that no other cookie is currently awaited
            if c.state.cookie.is_some() {
                return Err(DispatchError::peer(
                    "cookie_already_awaited",
                    anyhow!("A cookie is already being awaited"),
                ));
            }

            // Create a new channel and send the packet.
            let (tx, rx) = oneshot::channel();
            c.state.cookie = Some((C::KEY, tx));
            match c.phase() {
                Phase::Login => {
                    c.send(login::ServerCookieRequestPacket {
                        key: C::KEY.try_into().expect("infallible"),
                    })?;
                }
                Phase::Configuration => {
                    c.send(configuration::ServerCookieRequestPacket {
                        key: C::KEY.try_into().expect("infallible"),
                    })?;
                }
                _ => {
                    return Err(DispatchError::internal(
                        "cookie_invalid_phase",
                        anyhow!("Invalid phase"),
                    ));
                }
            };

            // Allow the connection to receive the cookie packet. In general, this allows packets other
            // than the cookie to be received. However, they will be blocked by the expect_step check.
            let gated = c.gated();
            c.release();
            Ok((rx, c.state.secret.clone(), gated))
        })?;

        // Wait for the cookie to be received. Then immediately reset the connection gated state.
        let payload = rx
            .await
            .map_err(|_| DispatchError::internal("cookie_error", anyhow!("Cookie never received")));
        if gated {
            self.with(|c| c.gate());
        }

        // Decode it with the secret. Some cookies require the secret to be present, while others ignore
        // it. After the cookie is received, the connection is gated again.
        let payload = match payload {
            Ok(Some(payload)) => payload,
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        };
        let cookie = C::decode(secret.as_deref(), payload.as_ref())
            .map_err(|err| DispatchError::internal("cookie_decode_error", err))?;
        Ok(cookie)
    }
}
