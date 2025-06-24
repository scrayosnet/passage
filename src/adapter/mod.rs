//! This module contains the adapter logic and the individual implementations of the adapters with
//! different responsibilities.

use std::net::AddrParseError;
use std::num::TryFromIntError;
use std::string;

mod refresh;
pub mod resourcepack;
pub mod status;
pub mod target_selection;
pub mod target_strategy;

#[cfg(feature = "grpc")]
pub mod proto {
    use crate::adapter::Error;
    use crate::adapter::Error::MissingData;
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
                return Err(MissingData { field: "address" });
            };
            let address =
                SocketAddr::from_str(&format!("{}:{}", raw_addr.hostname, raw_addr.port))?;

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
}

/// The internal error type for all errors related to the adapters and adapter communication.
///
/// This includes errors with the retrieval and parsing of adapter responses as well as problems
/// that occur during the initialization of the adapters. Those errors can correlate with the type
/// of adapter that is used but can also occur regardless of adapter choice.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The string could not be decoded as UTF-8, but all strings are expected to be UTF-8.
    #[error("could not decode UTF-8 string: {0}")]
    InvalidStringEncoding(#[from] string::FromUtf8Error),

    #[error("could not decode UUID: {0}")]
    InvalidUuidEncoding(#[from] uuid::Error),

    #[error("could not decode IP addr: {0}")]
    InvalidIpAddr(#[from] AddrParseError),

    #[error("could not use number as port: {0}")]
    InvalidPort(#[from] TryFromIntError),

    /// The value could not be serialized/deserialized from/to JSON.
    #[error("could not serialize/deserialize JSON: {0}")]
    InvalidJsonEncoding(#[from] serde_json::Error),

    /// The URL could not be successfully fetched.
    #[error("could not retrieve URL: {0}")]
    InvalidUrlResponse(#[from] reqwest::Error),

    /// The adapter could not be initialized because of a problem.
    #[error("failed to initialize {adapter_type} adapter: {cause}")]
    FailedInitialization {
        /// The type of adapter that failed to initialize.
        adapter_type: &'static str,
        /// The cause of the error that led to the failed initialization.
        #[source]
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A field was expected to be explicitly set but was missing in the adapter response.
    #[error("missing data field: {field}")]
    MissingData {
        /// The field that was missing.
        field: &'static str,
    },

    /// The creation of a gRPC client failed due to connection issues or wrong parameters.
    #[cfg(feature = "grpc")]
    #[error("could not create gRPC client: {0}")]
    GrpcClientFailed(#[from] tonic::transport::Error),

    /// The querying of some adapter over gRPC raised an error that was not expected.
    #[cfg(feature = "grpc")]
    #[error("failed to retrieve info through gRPC: {0}")]
    GrpcError(#[from] tonic::Status),

    /// Some mongodb error.
    #[cfg(feature = "mongodb")]
    #[error("failed mongodb operation: {0}")]
    MongodbError(#[from] mongodb::error::Error),

    /// Some mongodb bson error.
    #[cfg(feature = "mongodb")]
    #[error("failed mongodb bson operation: {0}")]
    BsonError(#[from] mongodb::bson::de::Error),

    /// No server could be found from the adapter, so the player will be disconnected.
    #[error("failed to retrieve target server: {message:#?}")]
    NoServerFound {
        /// The explicit message that should be used instead of the default message.
        message: Option<String>,
    },

    /// The adapter is currently unavailable.
    #[error("adapter is currently unavailable")]
    AdapterUnavailable,

    /// The server is not public, so the player cannot be connected.
    #[error("server is not public: {identifier}")]
    ServerNotPublic {
        /// The identifier of the server that is not public.
        identifier: String,
    },
}
