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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::versions;

    #[test]
    fn a_wire_failure_reads_as_itself() {
        // The codec adds nothing to a field that would not decode: the field-level message is
        // already the whole story, and wrapping it would only bury the field name.
        let wire = WireError::Utf8 { field: "host" };
        let error = CodecError::from(WireError::Utf8 { field: "host" });
        assert_eq!(error.to_string(), wire.to_string());
    }

    #[test]
    fn a_transport_failure_is_kept_apart_from_a_parsing_one() {
        // They are blamed differently: a malformed frame is the peer's doing, a broken socket is
        // nobody's, and the two answers come from this distinction.
        let error = CodecError::from(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        assert!(matches!(error, CodecError::Io(_)), "{error}");
    }

    #[test]
    fn sending_a_packet_that_does_not_exist_yet_names_both_sides_of_the_mismatch() {
        // A client waiting forever for something we never sent is the failure this prevents, so the
        // message has to say which packet and which version disagreed.
        let error = CodecError::PacketNotInVersion {
            packet: "Transfer",
            version: versions::V1_20_5,
        };
        let message = error.to_string();
        assert!(message.contains("Transfer"), "{message}");
        assert!(message.contains("766"), "{message}");
    }

    #[test]
    fn an_oversized_frame_says_how_far_over_it_was() {
        let error = CodecError::OversizedFrame {
            packet: "StatusResponse",
            length: 40_000,
            limit: 32_768,
        };
        let message = error.to_string();
        assert!(
            message.contains("40000") && message.contains("32768"),
            "{message}"
        );
    }
}
