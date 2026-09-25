mod handler;
mod utils;
mod state;

use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::error::Result;
use crate::router::handler::*;
use passage_core::router::Router;
use passage_core::server::Server;
use std::sync::Arc;
use tokio::net::TcpListener;
use crate::router::state::State;

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
            .on(on_login_login_start)?
            .on(on_login_encryption_response)?
            .on(on_login_login_acknowledged)?
            .on(on_configuration_client_information)?
            .on(on_configuration_keep_alive)?
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
