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
use std::sync::Arc;

pub use handler::*;
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
