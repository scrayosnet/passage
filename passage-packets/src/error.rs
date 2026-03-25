use crate::VarInt;

/// The adapter result type, wrapping the adapter error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The adapter error type for all errors related to the protocol communication.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An error occurred while reading or writing to the underlying byte stream.
    #[error("error reading or writing data: {0}")]
    Io(#[from] std::io::Error),

    /// Some `serde_json` error.
    #[error("failed to parse json: {0}")]
    Json(#[from] serde_json::error::Error),

    /// Some fastnbt error.
    #[error("failed to parse nbt: {0}")]
    Nbt(#[from] fastnbt::error::Error),

    /// The received value encoding cannot be mapped to an existing enum.
    #[error("illegal enum value encoding {value} for {kind}")]
    IllegalEnumValue {
        /// The enum kind which was parsed.
        kind: &'static str,

        /// The value that was received.
        value: VarInt,
    },

    /// The received packets is of an invalid length that we cannot process.
    #[error("illegal packets length {length} exceeds maximum of {limit} bytes")]
    IllegalPacketLength {
        /// The maximum allowed length of the packet.
        limit: usize,

        /// The actual length of the packet.
        length: usize,
    },

    /// The received packet ID does not match the expected ID (e.g., when parsing).
    #[error("illegal packet id {actual}, expected {expected:?}")]
    IllegalPacketId {
        /// The expected packet ID.
        expected: Vec<VarInt>,

        /// The actual packet ID.
        actual: VarInt,
    },

    /// The received packets ID does not match the expected ID (e.g., when parsing).
    #[error("illegal string UTF8 encoding: {0}")]
    IllegalStringEncoding(#[from] std::string::FromUtf8Error),

    /// The received token does not match the expected encoding (e.g., length).
    #[error("illegal token encoding")]
    IllegalTokenEncoding,
}
