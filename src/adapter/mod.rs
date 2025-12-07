//! This module contains the adapter logic and the individual implementations of the adapters with
//! different responsibilities.

mod refresh;
pub mod status;
pub mod target_selection;
pub mod target_strategy;

#[cfg(feature = "grpc")]
pub mod proto {
    use crate::adapter::{Error, MissingFieldError};
    use std::net::SocketAddr;
    use std::str::FromStr;

    tonic::include_proto!("scrayosnet.passage.adapter");

    impl From<&crate::adapter::target_selection::Target> for Target {
        fn from(value: &crate::adapter::target_selection::Target) -> Self {
            Self {
                identifier: value.identifier.clone(),
                address: Some(Address {
                    hostname: value.address.ip().to_string(),
                    port: u32::from(value.address.port()),
                }),
                meta: value
                    .meta
                    .iter()
                    .map(|(k, v)| MetaEntry {
                        key: k.clone(),
                        value: v.clone(),
                    })
                    .collect(),
            }
        }
    }

    impl TryFrom<&Target> for crate::adapter::target_selection::Target {
        type Error = Error;

        fn try_from(value: &Target) -> Result<Self, Self::Error> {
            let Some(raw_addr) = value.address.clone() else {
                return Err(Error::FailedParse {
                    adapter_type: "grpc_target",
                    cause: Box::new(MissingFieldError { field: "address" }),
                });
            };
            let address = SocketAddr::from_str(&format!("{}:{}", raw_addr.hostname, raw_addr.port))
                .map_err(|err| Error::FailedParse {
                    adapter_type: "grpc_target",
                    cause: err.into(),
                })?;

            Ok(Self {
                identifier: value.identifier.clone(),
                address,
                meta: value
                    .meta
                    .iter()
                    .map(|entry| (entry.key.clone(), entry.value.clone()))
                    .collect(),
            })
        }
    }

    impl TryFrom<Target> for crate::adapter::target_selection::Target {
        type Error = Error;

        fn try_from(value: Target) -> Result<Self, Self::Error> {
            let Some(raw_addr) = value.address.clone() else {
                return Err(Error::FailedParse {
                    adapter_type: "grpc_target",
                    cause: Box::new(MissingFieldError { field: "address" }),
                });
            };
            let address = SocketAddr::from_str(&format!("{}:{}", raw_addr.hostname, raw_addr.port))
                .map_err(|err| Error::FailedParse {
                    adapter_type: "grpc_target",
                    cause: err.into(),
                })?;

            Ok(Self {
                identifier: value.identifier,
                address,
                meta: value
                    .meta
                    .into_iter()
                    .map(|entry| (entry.key, entry.value))
                    .collect(),
            })
        }
    }
}

#[derive(thiserror::Error, Debug)]
#[error("missing data field: {field}")]
pub struct MissingFieldError {
    field: &'static str,
}

/// The internal error type for all errors related to the adapters and adapter communication.
///
/// This includes errors with the retrieval and parsing of adapter responses as well as problems
/// that occur during the initialization of the adapters. Those errors can correlate with the type
/// of adapter that is used but can also occur regardless of adapter choice.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The adapter could not be initialized because of a problem.
    #[error("failed to initialize {adapter_type} adapter: {cause}")]
    FailedInitialization {
        /// The type of adapter that failed.
        adapter_type: &'static str,
        /// The cause of the error.
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The adapter could not fetch some resource (e.g., server status) because of a problem.
    #[error("failed to fetch {adapter_type} resource: {cause}")]
    FailedFetch {
        /// The type of adapter that failed.
        adapter_type: &'static str,
        /// The cause of the error.
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The adapter could not parse some response resource (e.g., server status) because of a problem.
    #[error("failed to parse {adapter_type} resource: {cause}")]
    FailedParse {
        /// The type of adapter that failed.
        adapter_type: &'static str,
        /// The cause of the error.
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The adapter is currently unavailable.
    #[error("adapter is currently unavailable")]
    AdapterUnavailable {
        /// The type of adapter that failed.
        adapter_type: &'static str,
        /// The cause of the error.
        reason: &'static str,
    },
}
