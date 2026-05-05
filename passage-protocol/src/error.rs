#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The connection was closed, presumably by the client or server.
    #[error("The connection was closed (by the client)")]
    ConnectionClosed,

    /// No matching route was found.
    #[error("No matching route was found")]
    NoRouteFound,

    /// An error occurred during the invocation or communication of an adapter.
    #[error(transparent)]
    Adapter(#[from] passage_adapters::Error),

    /// An error occurred during the invocation or communication of an adapter.
    #[error(transparent)]
    Packet(#[from] passage_packets::Error),

    /// Some crypto/authentication request failed.
    #[error(transparent)]
    Crypto(#[from] crate::crypto::Error),

    /// Some cookie parsing failed.
    #[error(transparent)]
    Cookie(#[from] crate::cookie::Error),
}
