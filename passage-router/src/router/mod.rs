#[cfg(test)]
mod flow;
mod handler;
mod state;
mod utils;

use crate::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use passage_core::router::{Router, RouterError};
use std::sync::Arc;

use handler::*;
pub use state::State;

/// This crate uses enum dispatch to select the adapters at runtime.
pub type DynRoute = Route<
    DynStatusAdapter,
    Vec<DynDiscoveryActionAdapter>,
    DynAuthenticationAdapter,
    DynLocalizationAdapter,
>;

/// This crate uses enum dispatch to select the adapters at runtime.
///
/// The inner `Arc` is what lets a handler carry one route into an adapter call: it has to survive
/// the `.await`, and a borrow of the state cannot.
pub type DynRoutes = Arc<[Arc<DynRoute>]>;

/// Builds the router that drives the protocol: one handler per packet Passage answers.
///
/// The router is built once and shared by every connection, so this is the single place that
/// decides which packets exist. A packet with no handler ends the connection, which is why the last
/// two are registered at all -- a client may legitimately send them, and Passage simply has no
/// answer.
///
/// # Errors
///
/// Returns an error if two handlers claim the same packet, which is a programming error in the
/// list below rather than anything a deployment can cause.
pub fn router() -> Result<Router<State>, RouterError> {
    Ok(Router::builder()
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
        .build())
}
