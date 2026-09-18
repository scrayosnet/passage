use std::io;
use thiserror::Error;

/// The client result type, defaulting to [`ClientError`].
pub type Result<T, E = ClientError> = std::result::Result<T, E>;

/// Why a [`Client`](crate::client::Client) never reached the protocol.
///
/// Everything that happens *after* the socket is open is a
/// [`ConnectionError`](crate::connection::ConnectionError), reported in the
/// [`Outcome`](crate::connection::Outcome). This type covers only the two ways there is no
/// connection to report on.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The connector could not open a socket.
    #[error("failed to connect")]
    Connect(#[source] io::Error),

    /// A [`Layer`](crate::server::Layer) rejected the socket the connector opened.
    #[error("a layer rejected the connection")]
    Rejected,
}
