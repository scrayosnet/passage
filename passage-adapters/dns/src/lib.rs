pub mod discovery_adapter;
pub mod error;

// reexport errors types
#[allow(unused_imports)]
pub use error::*;

// reexport adapters
pub use discovery_adapter::{DnsDiscoveryAdapter, RecordType};
