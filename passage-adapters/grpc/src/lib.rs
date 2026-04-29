pub mod authentication_adapter;
pub mod discovery_action_adapter;
pub mod discovery_adapter;
pub mod error;
pub mod localization_adapter;
mod proto;
pub mod status_adapter;

// reexport errors types
pub use error::*;

pub use authentication_adapter::GrpcAuthenticationAdapter;
pub use discovery_action_adapter::GrpcDiscoveryActionAdapter;
// reexport adapters
pub use discovery_adapter::GrpcDiscoveryAdapter;
pub use localization_adapter::GrpcLocalizationAdapter;
pub use status_adapter::GrpcStatusAdapter;
