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

pub type Result<T, E = Error> = std::result::Result<T, E>;
