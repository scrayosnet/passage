use hickory_resolver::error::ResolveError;

#[derive(thiserror::Error, Debug)]
pub enum DnsError {
    #[error("DNS discovery lookup could not be performed: {cause}")]
    LookupFailed {
        /// The cause of the error.
        #[source]
        cause: Box<ResolveError>,
    },
}
