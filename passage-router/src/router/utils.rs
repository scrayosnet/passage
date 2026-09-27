use crate::cookie::Cookie;
use crate::crypto;
use crate::router::state::Step;
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
            .status
            .status(&self.client())
            .await?;
        Ok(status)
    }

    async fn authorize(&self, shared_secret: &[u8]) -> crate::Result<Profile, AdapterError> {
        let profile = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?
            .authentication
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
            .localization
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
        let (rx, secret) = self.with(|c| {
            // Only one cookie can be awaited at a time: the step a request returns to is the one it
            // interrupted, and a second request would lose it.
            if matches!(c.state.step, Step::Cookie { .. }) {
                return Err(DispatchError::internal(
                    "cookie_already_awaited",
                    anyhow!("a cookie is already being awaited"),
                ));
            }

            // Send the request first, so that a packet which does not encode leaves the step where
            // it was.
            match c.phase() {
                Phase::Login => c.send(login::ServerCookieRequestPacket {
                    key: C::KEY.try_into().expect("infallible"),
                })?,
                Phase::Configuration => c.send(configuration::ServerCookieRequestPacket {
                    key: C::KEY.try_into().expect("infallible"),
                })?,
                phase => {
                    return Err(DispatchError::internal(
                        "cookie_invalid_phase",
                        anyhow!("cookies cannot be requested in phase {phase:?}"),
                    ));
                }
            };

            // Wait for the answer in a step of its own, which is what stops anything else from
            // arriving meanwhile. The step it interrupted is what it returns to.
            let (tx, rx) = oneshot::channel();
            let resume = Box::new(c.state.swap_step(Step::Completed));
            c.state.step = Step::Cookie {
                key: C::KEY,
                sender: tx,
                resume,
            };
            Ok((rx, c.state.secret.clone()))
        })?;

        // Wait for the cookie. The handler that answers it puts the step back.
        let payload = rx
            .await
            .map_err(|_| DispatchError::internal("cookie_error", anyhow!("Cookie never received")));

        // Decode it with the secret. Some cookies require the secret to be present, while others
        // ignore it.
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
