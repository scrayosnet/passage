//! This crate provides gRPC interfaces for all adapters. These are primarily interesting for cases
//! in which Passage should provide some functionality that goes beyond the core scope. In general,
//! Passage should directly provide the building blocks for configuring it to every need. However,
//! in cases where a personalized functionality has to be provided, Passage provides two options:
//! - Firstly, writing custom adapters and building Passage from source with these or
//! - Secondly, using the gRPC interfaces provided by this crate to connect external services.
//!
//! The corresponding `.proto` files are located in the `/proto/adapter` directory. The gRPC adapters
//! are able to provide custom metadata for each found target. Service providers have to make sure to
//! follow the metadata convention for the best interoperability between (discovery action) adapters.
//!
//! When passing results, some adapters allow for rejecting the request and providing a reason language
//! key. This key is passed to the localization adapter to show users an appropriate error message.
//! It is, however, intended for external service adapters to be able to directly pass their message
//! in as the key.

pub mod authentication_adapter;
pub mod discovery_action_adapter;
pub mod discovery_adapter;
pub mod error;
pub mod localization_adapter;
mod proto;
pub mod status_adapter;

// reexport errors and adapters
pub use authentication_adapter::GrpcAuthenticationAdapter;
pub use discovery_action_adapter::GrpcDiscoveryActionAdapter;
pub use discovery_adapter::GrpcDiscoveryAdapter;
#[allow(unused_imports)]
pub use error::*;
pub use localization_adapter::GrpcLocalizationAdapter;
pub use status_adapter::GrpcStatusAdapter;
