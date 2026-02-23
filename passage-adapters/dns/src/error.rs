use hickory_resolver::error::ResolveError;

#[derive(thiserror::Error, Debug)]
pub enum DnsError {
    #[error("default_port is required when using A/AAAA records")]
    MissingPort,

    #[error("DNS discovery lookup could not be performed: {cause}")]
    LookupFailed {
        /// The cause of the error.
        #[source]
        cause: Box<ResolveError>,
    },
}
