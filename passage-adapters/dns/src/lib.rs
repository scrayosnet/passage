//! This crate provides DNS-based adapters.
//! - DNS Discovery is a discovery adapter that periodically resolves and caches targets by querying
//!   DNS SRV or A/AAAA records.

pub mod discovery_adapter;
pub mod error;

// reexport errors and adapters
pub use discovery_adapter::{DnsDiscoveryAdapter, RecordType};
#[allow(unused_imports)]
pub use error::*;
