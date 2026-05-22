use hickory_resolver::net::NetError;

/// Errors that can occur during DNS-based discovery.
#[derive(thiserror::Error, Debug)]
pub enum DnsError {
    /// The DNS resolver could not be constructed.
    #[error("DNS discoverer could not be built: {cause}")]
    BuildFailed {
        /// The cause of the error.
        #[source]
        cause: NetError,
    },

    /// A DNS lookup failed.
    #[error("DNS discovery lookup could not be performed: {cause}")]
    LookupFailed {
        /// The cause of the error.
        #[source]
        cause: NetError,
    },
}
