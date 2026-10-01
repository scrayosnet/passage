use crate::cookie::SessionCookie;
use crate::router::{DynRoute, DynRoutes};
use passage_adapters::{Client, Player};
use passage_core::common;
use passage_core::wire::Bytes;
use std::sync::Arc;
use tokio::sync::{Notify, oneshot};

/// Where the connection is in the protocol, and what only exists there.
///
/// Every handler names the step it answers, so a packet that arrives out of turn is refused by the
/// step rather than by each handler's own bookkeeping. What a step needs while it lasts is carried
/// in the step: it cannot be read too early, and it is gone once the step is.
#[derive(Debug)]
pub enum Step {
    /// Waiting for the handshake.
    Intention,

    /// Waiting for the status request.
    StatusRequest,

    /// Waiting for the ping the status response is answered with.
    StatusPingRequest,

    /// Waiting for the login start.
    LoginStart {
        /// Whether the client was transferred here, and so may present an auth cookie.
        transfer: bool,
    },

    /// A handler is doing something the client has to wait for: an adapter call, a session server
    /// round trip. Nothing may arrive until it is done, and the handler names the step it leaves in.
    Working {
        /// The task that is currently worked on. Read only through [`Debug`], which is what names
        /// the step in the error a packet arriving mid-task is refused with.
        #[allow(dead_code, reason = "read through the derived Debug impl")]
        task: &'static str,
    },

    /// A cookie was requested, and nothing else may arrive until it is answered.
    Cookie {
        /// The key that was asked for. A response for any other is not the one awaited.
        key: &'static str,

        /// Where the payload is delivered, which resumes the handler that asked.
        sender: oneshot::Sender<Option<Bytes>>,

        /// The step to return to once it has been answered.
        resume: Box<Step>,
    },

    /// Waiting for the encryption response.
    Encrypt {
        /// The token the client has to hand back, encrypted with our public key.
        verify_token: Bytes,

        /// Whether an auth cookie already vouched for the player.
        authenticated: bool,
    },

    /// Waiting for the login to be acknowledged.
    LoginAck,

    /// Keeping the client alive while a target is selected for it.
    Transfer {
        /// The ID of the last keep alive sent, until the client answers with it.
        keep_alive: Option<i64>,

        /// Notified when the client information packet arrives, which is what carries the locale.
        /// A disconnect message can only be localized after it.
        informed: Arc<Notify>,
    },

    /// Nothing more is expected from the client.
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

    /// The session of the client: the one it presented, or the one it was given for not having had
    /// any. Set once the connection is private enough to ask for the cookie, and `None` only before
    /// that -- every player that gets as far as the login success is in a session.
    pub(crate) session: Option<SessionCookie>,

    /// The secret used to sign the session cookie.
    pub(crate) secret: Option<Bytes>,

    /// The auth cookie expiry.
    pub(crate) auth_cookie_expiry: u64,
}

impl State {
    pub fn new(
        routes: DynRoutes,
        address: std::net::SocketAddr,
        secret: Option<Bytes>,
        auth_cookie_expiry: u64,
    ) -> Self {
        let client = Client {
            address,
            ..Client::default()
        };
        Self {
            routes,
            route_index: None,
            client,
            locale: None,
            player: Player::default(),
            step: Step::Intention,
            session: None,
            secret,
            auth_cookie_expiry,
        }
    }

    /// Moves to `step`, handing back the one it replaces.
    pub(crate) fn swap_step(&mut self, step: Step) -> Step {
        std::mem::replace(&mut self.step, step)
    }

    /// Sets the current step to `working`, recording the previous step. No handler should accept new
    /// packets while in the `working` step.
    pub(crate) fn set_working(&mut self, task: &'static str) -> Step {
        std::mem::replace(&mut self.step, Step::Working { task })
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
