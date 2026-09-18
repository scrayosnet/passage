use crate::version::ProtocolVersion;
use crate::wire::WireError;
use thiserror::Error;

/// The codec result type, defaulting to [`CodecError`].
pub type Result<T, E = CodecError> = std::result::Result<T, E>;

/// An error raised while framing or unframing the byte stream.
#[derive(Debug, Error)]
pub enum CodecError {
    /// A field of the frame could not be read or written.
    #[error(transparent)]
    Wire(#[from] WireError),

    /// The transport failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// A packet does not exist in the connection's protocol version, but we tried to send it.
    #[error("packet `{packet}` does not exist in protocol version {version}")]
    PacketNotInVersion {
        /// The packet we tried to send.
        packet: &'static str,
        /// The protocol version of the connection.
        version: ProtocolVersion,
    },

    /// A packet we built does not fit in a frame.
    ///
    /// The inbound path refuses oversized frames as a peer error; this is the same check on the way
    /// out. Emitting a length prefix that disagreed with the payload would desynchronise the peer
    /// with nothing to diagnose it from.
    #[error("encoded `{packet}` is {length} bytes, which exceeds the frame limit of {limit}")]
    OversizedFrame {
        /// The packet being encoded.
        packet: &'static str,
        /// The encoded length.
        length: usize,
        /// The configured frame limit.
        limit: usize,
    },
}
