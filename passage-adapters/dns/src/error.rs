use passage_adapters::Error as AdapterError;

/// Converts DNS errors to adapter errors.
pub fn dns_error(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> AdapterError {
    AdapterError::FailedFetch {
        adapter_type: "dns_discovery",
        cause: err.into(),
    }
}

/// Converts DNS initialization errors to adapter errors.
pub fn dns_init_error(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> AdapterError {
    AdapterError::FailedInitialization {
        adapter_type: "dns_discovery",
        cause: err.into(),
    }
}
