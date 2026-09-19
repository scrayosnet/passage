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

    /// A [`Layer`](crate::router::Layer) rejected the socket the connector opened.
    #[error("a layer rejected the connection")]
    Rejected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn a_failed_dial_keeps_the_reason_it_failed() {
        // The message says what we were doing; the source says what went wrong. Flattening the two
        // would lose "connection refused", which is the only part worth acting on.
        let error = ClientError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused));
        assert_eq!(error.to_string(), "failed to connect");
        let source = error.source().expect("a cause");
        assert!(source.to_string().contains("refused"), "{source}");
    }

    #[test]
    fn a_refusal_has_nothing_to_add() {
        // A layer holding the reason is the point of a layer: it says nothing to anyone, and the
        // client invents nothing on its behalf.
        assert!(ClientError::Rejected.source().is_none());
    }
}
