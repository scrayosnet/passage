use crate::cookie::SessionCookie;
use crate::router::{DynRoute, DynRoutes};
use passage_adapters::{Client, Player};
use passage_core::common;
use passage_core::wire::Bytes;
use std::sync::Arc;
use tokio::sync::{Notify, oneshot};

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Intention,
    StatusRequest,
    StatusPingRequest,
    LoginStart {
        transfer: bool,
    },
    Encrypt {
        verify_token: Bytes,
        authenticated: bool,
    },
    LoginAck,
    Transfer,
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

// TODO replace prop access with methods
pub struct State {
    /// The shared set of routes.
    pub(crate) routes: DynRoutes,

    /// The index of the currently selected route.
    pub(crate) route_index: Option<usize>,

    /// The client information. Set after the intention packet is received.
    pub(crate) client: Client,

    /// The locale of the client.
    pub(crate) locale: Option<String>,

    /// The player information. Set after the login start and verified afterward.
    pub(crate) player: Player,

    /// The current step of the connection. This binds the client to the server protocol.
    pub(crate) step: Step,

    /// The key of the cookie that was requested, and where to deliver it. A response for any other
    /// key is not the one that was asked for.
    pub(crate) cookie: Option<(&'static str, oneshot::Sender<Option<Bytes>>)>,

    /// The session the client presented, if it had one. A client without one is given a new session
    /// when it is transferred.
    pub(crate) session: Option<SessionCookie>,

    /// Notified when the client information packet arrives, which is what carries the locale. A
    /// disconnect message can only be localized after it.
    pub(crate) informed: Arc<Notify>,

    /// The secret used to sign the session cookie.
    pub(crate) secret: Option<Bytes>,

    /// The auth cookie expiry.
    pub(crate) auth_cookie_expiry: u64,

    /// The last sent keep alive id.
    pub(crate) keep_alive_id: Option<i64>,
}

impl State {
    pub fn new(
        routes: DynRoutes,
        address: std::net::SocketAddr,
        secret: Option<Bytes>,
        auth_cookie_expiry: u64,
    ) -> Self {
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
            session: None,
            informed: Arc::new(Notify::new()),
            secret,
            auth_cookie_expiry,
            keep_alive_id: None,
        }
    }

    pub(crate) fn find_route(&mut self, server_address: &str) {
        self.route_index = self
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.hostname.is_match(server_address))
            .map(|(index, _)| index);
    }

    /// The selected route, as something a handler can carry into an adapter call.
    pub(crate) fn route(&self) -> Option<Arc<DynRoute>> {
        self.route_index
            .map(|index| Arc::clone(&self.routes[index]))
    }
}
