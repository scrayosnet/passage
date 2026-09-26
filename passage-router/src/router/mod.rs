#[cfg(test)]
mod flow;
mod handler;
mod state;
mod utils;

use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::error::Result;
use crate::router::handler::*;
use crate::router::state::State;
use passage_core::router::Router;
use passage_core::server::Server;
use passage_core::wire::Bytes;
use std::sync::Arc;
use tokio::net::TcpListener;

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

pub struct Passage {
    routes: DynRoutes,

    /// The secret the auth cookie is signed with. Without one, a transferring client is
    /// authenticated against the authentication adapter like any other.
    secret: Option<Bytes>,

    /// How long an auth cookie stays valid, in seconds.
    auth_cookie_expiry: u64,
}

impl Passage {
    pub fn new(routes: DynRoutes, secret: Option<Bytes>, auth_cookie_expiry: u64) -> Self {
        Self {
            routes,
            secret,
            auth_cookie_expiry,
        }
    }

    // TODO pass adapters or config?

    fn router(&self) -> Result<Arc<Router<State>>> {
        let router = Router::builder()
            // general handlers
            .on_open(on_open)
            // packet handlers
            .on(on_handshake_intention_packet)?
            .on(on_status_request_packet)?
            .on(on_status_ping_request_packet)?
            .on(on_login_login_start)?
            .on(on_login_encryption_response)?
            .on(on_login_login_acknowledged)?
            .on(on_login_cookie_response)?
            .on(on_configuration_client_information)?
            .on(on_configuration_keep_alive)?
            .on(on_configuration_cookie_response)?
            // packets the client may send that Passage has no answer for. Without a handler they
            // would end the connection, which is what `UnknownPolicy::Reject` is for.
            .on(on_configuration_custom_payload)?
            .on(on_configuration_resource_pack)?
            .build();
        Ok(Arc::new(router))
    }

    pub async fn serve(self) -> Result<()> {
        let router = self.router()?;
        let listener = TcpListener::bind("").await?;
        Server::new(listener)
            .dispatch(router)
            .state(move |addr| {
                State::new(
                    self.routes.clone(),
                    *addr,
                    self.secret.clone(),
                    self.auth_cookie_expiry,
                )
            })
            .serve()
            .await;
        Ok(())
    }
}
