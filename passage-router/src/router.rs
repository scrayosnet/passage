use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::error::Result;
use anyhow::{Context, anyhow};
use passage_adapters::{Client, LocalizationAdapter, Player, ServerStatus, StatusAdapter};
use passage_core::connection::{Conn, ConnRef, ConnectionError, DispatchError};
use passage_core::packet::handshake::ClientIntentionPacket;
use passage_core::packet::login::{ClientLoginStartPacket, ServerDisconnectPacket};
use passage_core::packet::status::{
    ClientPingRequestPacket, ClientStatusRequestPacket, ServerPongResponsePacket,
    ServerStatusResponsePacket,
};
use passage_core::router::Router;
use passage_core::server::{Listener, Server};
use passage_core::{common, versions, Phase};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{debug, info};
use passage_core::packet::{configuration, login};
use passage_core::wire::{ByteString, Bytes};
use crate::cookie::{SessionCookie, AUTH_COOKIE_KEY, SESSION_COOKIE_KEY, Cookie, AuthCookie};
use crate::crypto;

/// This crate uses enum dispatch to select the adapters at runtime.
type DynRoute = Route<
    DynStatusAdapter,
    DynDiscoveryActionAdapter,
    DynAuthenticationAdapter,
    DynLocalizationAdapter,
>;

/// This crate uses enum dispatch to select the adapters at runtime.
///
/// The inner `Arc` is what lets a handler carry one route into an adapter call: it has to survive
/// the `.await`, and a borrow of the state cannot.
type DynRoutes = Arc<[Arc<DynRoute>]>;

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Intention,
    StatusRequest,
    StatusPingRequest,
    LoginStart { transfer: bool },
    Encrypt { verify_token: Bytes },
    Completed,
}

impl From<common::State> for Step {
    fn from(value: common::State) -> Self {
        match value {
            common::State::Status => Step::StatusRequest,
            common::State::Login => Step::LoginStart { transfer: false },
            common::State::Transfer => Step::LoginStart { transfer: true },
        }
    }
}

pub struct State {
    /// The shared set of routes.
    routes: DynRoutes,

    /// The index of the currently selected route.
    route_index: Option<usize>,

    /// The client information. Set after the intention packet is received.
    client: Client,

    /// The locale of the client.
    locale: Option<String>,

    /// The player information. Set after the login start and verified afterward.
    player: Player,

    /// The current step of the connection. This binds the client to the server protocol.
    step: Step,

    /// A receiver for the next cookie response.
    cookie: Option<oneshot::Sender<Option<Bytes>>>,

    /// The secret used to sign the session cookie.
    secret: Option<Bytes>,

    /// The auth cookie expiry.
    auth_cookie_expiry: u64,
}

impl State {
    pub fn new(routes: DynRoutes, address: std::net::SocketAddr) -> Self {
        let mut client = Client::default();
        client.address = address;
        Self {
            routes,
            route_index: None,
            client,
            locale: None,
            player: Player::default(),
            step: Step::Intention,
            cookie: None,
            secret: None,
            auth_cookie_expiry: 64,
        }
    }

    pub fn find_route(&mut self, server_address: &str) {
        self.route_index = self
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.hostname.is_match(server_address))
            .map(|(index, _)| index);
    }

    /// The selected route, as something a handler can carry into an adapter call.
    pub fn route(&self) -> Option<Arc<DynRoute>> {
        self.route_index
            .map(|index| Arc::clone(&self.routes[index]))
    }
}

trait ConnRefExt {
    fn status(&self) -> impl Future<Output = Result<ServerStatus, DispatchError>>;
    fn localize(
        &self,
        key: &str,
        params: &[(&'static str, String)],
    ) -> impl Future<Output = Result<ByteString, DispatchError>>;

    fn cookie<C: Cookie>(&self) -> impl Future<Output = Result<Option<C>, DispatchError>>;
}

impl ConnRefExt for ConnRef<'_, State> {
    async fn status(&self) -> Result<ServerStatus, DispatchError> {
        let (route, client) = self.with(|c| (c.state.route(), c.state.client.clone()));
        let Some(route) = route else {
            return Err(DispatchError::peer("no_route", anyhow!("No route found")));
        };
        route
            .status_adapter
            .status(&client)
            .await
            .map_err(|err| DispatchError::internal("status_error", err))?
            .ok_or(DispatchError::internal(
                "no_status",
                anyhow!("No status found for the client"),
            ))
    }

    async fn localize(
        &self,
        key: &str,
        params: &[(&'static str, String)],
    ) -> Result<ByteString, DispatchError> {
        let (route, locale) = self.with(|c| (c.state.route(), c.state.locale.clone()));
        let Some(route) = route else {
            return Err(DispatchError::peer("no_route", anyhow!("No route found")));
        };
        let message = route
            .localization_adapter
            .localize(locale.as_deref(), key, params)
            .await
            .map_err(|err| DispatchError::internal("localize_error", err))?
            .try_into()
            .expect("infallible");
        Ok(message)
    }

    async fn cookie<C: Cookie>(&self) -> Result<Option<C>, DispatchError> {
        let (rx, secret) = self.with(|c| {
            // Ensure that no other cookie is currently awaited
            if c.state.cookie.is_some() {
                return Err(DispatchError::peer(
                    "cookie_already_awaited",
                    anyhow!("A cookie is already being awaited"),
                ));
            }

            // Create a new channel and send the packet.
            let (tx, rx) = oneshot::channel();
            c.state.cookie = Some(tx);
            match c.phase() {
                Phase::Login => {
                    c.send(login::ServerCookieRequestPacket { key: C::KEY.try_into().expect("infallible") })?;
                }
                Phase::Configuration => {
                    c.send(configuration::ServerCookieRequestPacket { key: C::KEY.try_into().expect("infallible") })?;
                }
                _ => return Err(DispatchError::internal("cookie_invalid_phase", anyhow!("Invalid phase"))),
            };

            // Allow the connection to receive the cookie packet. In general, this allows packets other
            // than the cookie to be received. However, they will be blocked by the expect_step check.
            c.release();
            Ok((rx, c.state.secret.clone()))
        })?;

        // Wait for the cookie to be received. Then, decode it with the secret. Some cookies require
        // the secret to be present, while others ignore it. After the cookie is received, the connection
        // is gated again.
        let payload = rx.await
            .map_err(|_| DispatchError::internal("cookie_error", anyhow!("Cookie never received")))?;
        self.with(|c| c.gate());
        let Some(payload) = payload else {
            return Ok(None)
        };
        let cookie = C::decode(secret.as_deref(), payload.as_ref())
            .map_err(|err| DispatchError::internal("cookie_decode_error", err))?;
        Ok(cookie)
    }
}

pub struct Passage {
    routes: DynRoutes,
}

impl Passage {
    pub fn new(routes: DynRoutes) -> Self {
        Self { routes }
    }

    // TODO pass adapters or config?

    fn router(&self) -> Result<Arc<Router<State>>> {
        let router = Router::builder()
            // general handlers
            .on_open(on_open)
            .on_tick(on_tick)
            .on_error(on_error)
            // packet handlers
            .on(on_handshake_intention_packet)?
            .on(on_status_request_packet)?
            .on(on_status_ping_request_packet)?
            .build();
        Ok(Arc::new(router))
    }

    pub async fn serve(self) -> Result<()> {
        let router = self.router()?;
        let listener = TcpListener::bind("").await?;
        Server::new(listener)
            .dispatch(router)
            .state(move |addr| State::new(self.routes.clone(), addr.clone()))
            .serve()
            .await;
        Ok(())
    }
}

// handlers

fn on_open(conn: ConnRef<'_, State>) -> Result<(), DispatchError> {
    let _ = conn;
    Ok(())
}

async fn on_tick(conn: ConnRef<'_, State>) -> Result<(), DispatchError> {
    let _ = conn;
    Ok(())
}

fn on_error(conn: ConnRef<'_, State>, error: &mut ConnectionError) -> Result<(), DispatchError> {
    let _ = (conn, error);
    Ok(())
}

/// Fails unless the connection is at the step this handler answers.
///
/// The step and the route are written together and read together, which is the reason they are
/// plain fields under one lend rather than separately shared: a caller must never see the step
/// advanced while the route it implies is still missing.
fn expect_step(conn: &Conn<State>, step: Step) -> Result<(), DispatchError> {
    if conn.state.step == step {
        return Ok(());
    }
    Err(DispatchError::peer(
        "unexpected_step",
        anyhow!("Expected step `{step:?}`, got `{:?}`", conn.state.step),
    ))
}

async fn on_handshake_intention_packet(
    conn: ConnRef<'_, State>,
    packet: ClientIntentionPacket,
) -> Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step(c, Step::Intention)?;
        c.state.step = packet.next_state.into();

        // Update the connection state.
        c.set_version(packet.protocol_version);
        c.set_phase(packet.next_state.into());

        // Update the custom state.
        c.state.find_route(&packet.server_address);
        c.state.client.protocol_version = packet.protocol_version;
        c.state.client.server_address = packet.server_address;
        c.state.client.server_port = packet.server_port;
        Ok(())
    })
}

async fn on_status_request_packet(
    conn: ConnRef<'_, State>,
    _packet: ClientStatusRequestPacket,
) -> Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step(c, Step::StatusRequest)?;
        c.state.step = Step::StatusPingRequest;
        Ok::<(), DispatchError>(())
    })?;

    // Query the status adapter based on the selected route. Then, send the status response to the
    // client.
    let status = conn.status().await?;
    let packet = ServerStatusResponsePacket::try_from(&status)
        .map_err(|err| DispatchError::internal("status_encode_error", err))?;
    conn.send(packet)?;
    Ok(())
}

async fn on_status_ping_request_packet(
    conn: ConnRef<'_, State>,
    packet: ClientPingRequestPacket,
) -> Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step(c, Step::Intention)?;
        c.state.step = Step::Completed;

        // Send the pong response and close the connection.
        c.send(ServerPongResponsePacket {
            payload: packet.payload,
        })?;
        c.close();
        Ok(())
    })
}

async fn on_login_login_start(
    conn: ConnRef<'_, State>,
    packet: ClientLoginStartPacket,
) -> Result<(), DispatchError> {
    let (transfer, client_address, cookie_expiry) = conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step(c, Step::LoginStart { .. })?;

        // Update the custom state.
        c.state.player.name = packet.name.to_string();
        c.state.player.id = packet.uuid;

        let transfer = c.state.step == Step::LoginStart { transfer: true };
        let client_address = c.state.client.address;
        let auth_cookie_expiry = c.state.auth_cookie_expiry;
        Ok((transfer, client_address, auth_cookie_expiry))
    })?;

    // Ensure that the client supports the transfer packet. Otherwise, close the connection with a
    // disconnect packet. The packet will tell the client which minecraft version they should use.
    if conn.version() <= versions::V1_20_5 {
        let status = conn.status().await?;
        let reason = conn
            .localize("disconnect_unsupported", &[("preferred", status.version.name)])
            .await?;
        conn.send(ServerDisconnectPacket { reason })?;
        conn.close();
        return Ok(());
    };

    // Handle transfer by checking the auth cookie.
    let mut authenticated = false;
    'transfer: {
        let has_secret = conn.with(|c| c.state.secret.is_some());
        if has_secret && transfer {
            let auth_cookie = conn.cookie::<AuthCookie>()
                .await
                .context("Failed to get the auth cookie")?;
            let Some(auth_cookie) = auth_cookie else {
                break 'transfer;
            };

            // Check that the auth cookie is valid.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time error")
                .as_secs();
            if auth_cookie.client_addr.ip() != client_address.ip() || (auth_cookie.timestamp + cookie_expiry) < now {
                debug!("invalid auth cookie payload received, skipping auth cookie");
                break 'transfer;
            }

            // Update the auth state
            authenticated = true;
            conn.with(|c| {
                c.state.player.name = auth_cookie.user_name.to_string();
                c.state.player.id = auth_cookie.user_id;
                c.state.player.profile_properties = auth_cookie.profile_properties;
            })
        }
    }

    // Send the encryption message.
    let verify_token = crypto::generate_token()
        .map_err(|err| DispatchError::internal("verify_token_error", err))?;
    conn.with(|c| {
        c.send(login::ServerEncryptionRequestPacket {
            server_id: ByteString::new(),
            public_key: crypto::ENCODED_PUB.clone(),
            verify_token: Default::default(),
            should_authenticate: !authenticated,
        })?;
        c.state.step = Step::Encrypt { verify_token };
        Ok(())
    })?;
    Ok(())
}


async fn on_login_encryption_response(
    conn: ConnRef<'_, State>,
    packet: login::ClientEncryptionResponsePacket,
) -> Result<(), DispatchError> {
    let verify_token = conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step(c, Step::Encrypt { verify_token })?;
        Ok(verify_token)
    })?;

    // decrypt the shared secret and verify the token
    let shared_secret = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.shared_secret)
        .map_err(|err| DispatchError::internal("shared_secret_decrypt_error", err))?;
    let decrypted_verify_token = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.verify_token)
        .map_err(|err| DispatchError::internal("verify_token_decrypt_error", err))?;

    // verify the token is correct
    debug!("verifying verify token");
    if !crypto::verify_token(verify_token, &decrypted_verify_token) {
        let message = conn.localize("disconnect_invalid_session", &[]).await?;
        info!("received invalid verify token, closing connection");
        conn.with(|c| {
            c.state.step = Step::Completed;
            c.send(login::ServerDisconnectPacket::text(message))?;
            c.close();
            Ok(())
        })?;
        return Ok(());
    }

    conn.with(|c| {
        c.encrypt(todo!(""));
        c.state.step = todo!("");
        // TODO authenticate if unauthenticated, then update profile
        // TODO Then send login success
        // TODO Then start target selection
    });

    Ok(())
}
