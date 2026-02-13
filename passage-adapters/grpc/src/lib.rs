mod discovery_adapter;
pub mod error;
mod proto;
pub mod status_adapter;
mod strategy_adapter;

// reexport errors types
pub use error::*;

// reexport adapters
pub use discovery_adapter::GrpcDiscoveryAdapter;
pub use status_adapter::GrpcStatusAdapter;
pub use strategy_adapter::GrpcStrategyAdapter;
