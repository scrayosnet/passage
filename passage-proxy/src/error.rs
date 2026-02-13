use crate::crypto;
use passage_packets::VarInt;
use std::io::ErrorKind;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An unrecognized io error. All expected io errors are
    #[error("unexpected io error: {0}")]
    InternalIo(std::io::Error),

    /// The JSON version of a packet content could not be encoded.
    #[error("invalid struct for JSON (encoding problem)")]
    Json(#[from] serde_json::Error),

    /// Some crypto/authentication request failed.
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] crypto::Error),

    #[error("authentication request failed: {0}")]
    AuthRequestFailed(#[from] reqwest::Error),

    /// Keep-alive was not received.
    #[error("Missed keep-alive")]
    MissedKeepAlive,

    #[error("invalid verification token received")]
    InvalidVerifyToken,

    /// No target was found for the user.
    #[error("No target was found for the user")]
    NoTargetFound,

    /// The connection was closed, presumably by the client.
    #[error("The connection was closed (by the client)")]
    ConnectionClosed(std::io::Error),

    /// The received passage-packets is of an invalid length that we cannot process.
    #[error("illegal passage-packets length")]
    IllegalPacketLength,

    /// The received value index cannot be mapped to an existing enum.
    #[error("illegal enum value index for {kind}: {value}")]
    IllegalEnumValue {
        /// The enum kind which was parsed.
        kind: &'static str,
        /// The value that was received.
        value: VarInt,
    },

    /// The received passage-packets ID is not mapped to an expected packet.
    #[error("unexpected packet id received {0}")]
    UnexpectedPacketId(VarInt),

    /// The JSON response of a packet is incorrectly encoded (not UTF-8).
    #[error("invalid response body (invalid encoding)")]
    InvalidEncoding,

    /// Some array conversion failed.
    #[error("could not convert into array")]
    ArrayConversionFailed,

    /// Some fastnbt error.
    #[error("failed to parse nbt: {0}")]
    Nbt(#[from] passage_packets::fastnbt::error::Error),

    /// An error occurred during the invocation or communication of an adapter.
    #[error("failed to invoke adapter: {0}")]
    AdapterError(#[from] passage_adapters::Error),
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        match value.kind() {
            ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::NetworkDown
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::WriteZero
            | ErrorKind::UnexpectedEof => Error::ConnectionClosed(value),
            _ => Error::InternalIo(value),
        }
    }
}

impl From<passage_packets::Error> for Error {
    fn from(value: passage_packets::Error) -> Self {
        match value {
            passage_packets::Error::Io(err) => err.into(),
            passage_packets::Error::IllegalPacketLength => Error::IllegalPacketLength,
            passage_packets::Error::IllegalEnumValue { kind, value } => {
                Error::IllegalEnumValue { kind, value }
            }
            passage_packets::Error::IllegalPacketId { actual, .. } => {
                Error::UnexpectedPacketId(actual)
            }
            passage_packets::Error::InvalidEncoding => Error::InvalidEncoding,
            passage_packets::Error::ArrayConversionFailed => Error::ArrayConversionFailed,
            passage_packets::Error::Json(err) => Error::Json(err),
            passage_packets::Error::Nbt(err) => Error::Nbt(err),
        }
    }
}

impl Error {
    pub fn as_label(&self) -> &'static str {
        match self {
            Error::MissedKeepAlive => "missed-keep-alive",
            Error::NoTargetFound => "no-target-found",
            Error::ConnectionClosed(_) => "connection-closed",
            Error::IllegalPacketLength
            | Error::IllegalEnumValue { .. }
            | Error::UnexpectedPacketId { .. }
            | Error::InvalidEncoding => "protocol-error",
            _ => "internal-error",
        }
    }
}
