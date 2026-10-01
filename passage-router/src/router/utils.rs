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
use sha2::{Digest, Sha256};
use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{Span, debug, field, instrument, trace, warn};
use uuid::Uuid;

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

    #[instrument(
        level = "info",
        skip_all,
        fields(
            otel.kind = "client",
            adapter = field::Empty,
        ),
    )]
    async fn status(&self) -> crate::Result<Option<ServerStatus>, AdapterError> {
        let route = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?;
        Span::current().record("adapter", field::display(&route.status));
        let status = route.status.status(&self.client()).await?;
        Ok(status)
    }

    #[instrument(
        level = "info",
        skip_all,
        fields(
            otel.kind = "client",
            adapter = field::Empty,
            player = %self.player().name,
        ),
    )]
    async fn authorize(&self, shared_secret: &[u8]) -> crate::Result<Profile, AdapterError> {
        let route = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?;
        Span::current().record("adapter", field::display(&route.authentication));
        let profile = route
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

    #[instrument(
        level = "info",
        skip_all,
        fields(
            otel.kind = "client",
            key = key,
        ),
    )]
    async fn localize(&self, key: &str, params: &[(&'static str, String)]) -> ByteString {
        let Some(route) = self.route() else {
            debug!("no route to localize with, falling back to the key");
            return ByteString::from(key);
        };

        let message = route
            .localization
            .localize(self.locale().as_deref(), key, params)
            .await;
        match message {
            Ok(message) => message.into(),
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

    #[instrument(
        level = "info",
        skip_all,
        fields(
            otel.kind = "client",
            pipeline = field::Empty,
            player = %self.player().name,
            target = field::Empty,
        ),
    )]
    async fn target(&self) -> crate::Result<Target, AdapterError> {
        let route = self
            .route()
            .ok_or(AdapterError::reject_reason("router", "No route selected"))?;
        let span = Span::current();
        span.record(
            "pipeline",
            route
                .discovery
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" -> "),
        );
        let target = route.select(&self.client(), &self.player()).await?;
        span.record("target", field::display(&target.identifier));
        Ok(target)
    }

    #[instrument(level = "debug", skip_all, fields(key = C::KEY))]
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
                Phase::Login => c.send(login::ServerCookieRequestPacket { key: C::KEY.into() })?,
                Phase::Configuration => {
                    c.send(configuration::ServerCookieRequestPacket { key: C::KEY.into() })?
                }
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
        trace!("waiting for the client to answer the cookie request");
        let payload = rx
            .await
            .map_err(|_| DispatchError::internal("cookie_error", anyhow!("Cookie never received")));

        // Decode it with the secret. Some cookies require the secret to be present, while others
        // ignore it.
        let payload = match payload {
            Ok(Some(payload)) => payload,
            Ok(None) => {
                debug!("the client holds no such cookie");
                return Ok(None);
            }
            Err(err) => return Err(err),
        };
        let cookie = C::decode(secret.as_deref(), payload.as_ref())
            .map_err(|err| DispatchError::internal("cookie_decode_error", err))?;
        debug!(
            length = payload.len(),
            valid = cookie.is_some(),
            "the client answered with a cookie",
        );
        Ok(cookie)
    }
}

/// The anonymized form of a player's identity, for the `user.hash` attribute: the SHA-256 of the
/// UUID in the hyphenated, lowercase form the client itself uses, hex-encoded in lowercase.
///
/// It is what lets a player be followed across traces by someone who may not see the UUID, which is
/// only worth anything if everyone producing it agrees on the exact bytes being hashed -- the string
/// form and not the 16 raw bytes, hyphenated and not compact, lowercase on both sides of the hash.
pub(crate) fn user_hash(id: &Uuid) -> String {
    let mut buffer = Uuid::encode_buffer();
    let id = id.hyphenated().encode_lower(&mut buffer);

    let mut hash = String::with_capacity(Sha256::output_size() * 2);
    for byte in Sha256::digest(id.as_bytes()) {
        write!(hash, "{byte:02x}").expect("writing into a String cannot fail");
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_hash_is_the_lowercase_hex_sha256_of_the_hyphenated_uuid() {
        // Pinned against a value computed outside this crate. The attribute is only useful if it
        // matches what every other producer writes for the same player, so the shape of the input
        // -- hyphenated, lowercase, as a string -- is part of the contract and not an detail.
        let id = Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").expect("a uuid");
        assert_eq!(
            user_hash(&id),
            "992225b786aba63f4ab4e664e44a8a6141042cca540cd7fe2423823d5077b2b8",
        );
    }

    #[test]
    fn a_user_hash_does_not_depend_on_how_the_uuid_was_written() {
        // The same player, parsed from the compact and the uppercase form: a hash that followed the
        // spelling rather than the value would split one player into three.
        let hyphenated = Uuid::parse_str("069a79f4-44e9-4726-a5be-fca90e38aaf5").expect("a uuid");
        let compact = Uuid::parse_str("069a79f444e94726a5befca90e38aaf5").expect("a uuid");
        let upper = Uuid::parse_str("069A79F4-44E9-4726-A5BE-FCA90E38AAF5").expect("a uuid");

        assert_eq!(user_hash(&hyphenated), user_hash(&compact));
        assert_eq!(user_hash(&hyphenated), user_hash(&upper));
    }
}
