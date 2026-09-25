/// The internal error type for all errors related to the adapters and adapter communication.
///
/// This includes errors with the retrieval and parsing of adapter responses as well as problems
/// that occur during the initialization of the adapters. Those errors can correlate with the type
/// of adapter that is used but can also occur regardless of adapter choice.
#[derive(thiserror::Error, Debug)]
pub enum AdapterError {
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

    /// The adapter rejected the request because for some reason.
    #[error("request rejected {adapter_type}: {reason:?}")]
    Rejected {
        /// The type of adapter that failed.
        adapter_type: &'static str,

        /// The reason for the rejection. This is a localizable message key.
        reason: Option<String>,
    },
}

impl AdapterError {
    pub fn is_rejected(&self) -> bool {
        matches!(self, AdapterError::Rejected { .. })
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            AdapterError::Rejected { reason, .. } => reason.as_deref(),
            _ => None,
        }
    }

    pub fn reject(adapter_type: &'static str) -> Self {
        AdapterError::Rejected {
            adapter_type,
            reason: None,
        }
    }

    pub fn reject_reason(adapter_type: &'static str, reason: impl Into<String>) -> Self {
        AdapterError::Rejected {
            adapter_type,
            reason: Some(reason.into()),
        }
    }
}

/// Constructs a [`AdapterError::Rejected`] without a reason.
pub fn reject(adapter_type: &'static str) -> AdapterError {
    AdapterError::Rejected {
        adapter_type,
        reason: None,
    }
}

/// Constructs a [`AdapterError::Rejected`] with a localizable message key as the reason.
pub fn reject_reason(adapter_type: &'static str, reason: impl Into<String>) -> AdapterError {
    AdapterError::Rejected {
        adapter_type,
        reason: Some(reason.into()),
    }
}

/// Convenience alias for `Result<T, passage_adapters::Error>`.
pub type Result<T, E = AdapterError> = std::result::Result<T, E>;
