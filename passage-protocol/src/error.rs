use crate::crypto;
use passage_packets::VarInt;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    // TODO move cookie errors into a separate module?
    /// Failed to encode or decode a cookie.
    #[error("failed to en-/decode {cookie} cookie")]
    Cookie {
        /// The name of the cookie.
        cookie: &'static str,

        /// The source error.
        #[source]
        source: serde_json::Error,
    },

    /// The connection was closed, presumably by the client or server.
    #[error("The connection was closed (by the client)")]
    ConnectionClosed,

    /// An error occurred during the invocation or communication of an adapter.
    #[error(transparent)]
    Adapter(#[from] passage_adapters::Error),

    /// An error occurred during the invocation or communication of an adapter.
    #[error(transparent)]
    Packet(#[from] passage_packets::Error),

    /// Some crypto/authentication request failed.
    #[error(transparent)]
    Crypto(#[from] crypto::Error),
}

impl Error {
    /// Builds an error from a `passage_packets::Error::IllegalPacketId`.
    pub fn illegal_packet_id(expected: Vec<VarInt>, actual: VarInt) -> Self {
        Self::Packet(passage_packets::Error::IllegalPacketId { expected, actual })
    }
}
