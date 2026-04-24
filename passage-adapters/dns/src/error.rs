use hickory_resolver::net::NetError;

#[derive(thiserror::Error, Debug)]
pub enum DnsError {
    #[error("DNS discoverer could not be built: {cause}")]
    BuildFailed {
        /// The cause of the error.
        #[source]
        cause: NetError,
    },
    #[error("DNS discovery lookup could not be performed: {cause}")]
    LookupFailed {
        /// The cause of the error.
        #[source]
        cause: NetError,
    },
}
