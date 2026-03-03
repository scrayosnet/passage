use crate::crypto;
use passage_packets::VarInt;
use std::string::FromUtf8Error;

// TODO better differentiate between client and server errors?
#[derive(thiserror::Error, Debug)]
pub enum Error {
    // TODO specify error cause (only from cookie paring right)?
    /// The JSON version of a packet content could not be encoded.
    #[error("invalid struct for JSON (encoding problem)")]
    Json(#[from] serde_json::Error),

    /// Some crypto/authentication request failed.
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] crypto::Error),

    #[error("no profile found")]
    Unauthenticated,

    /// Keep-alive was not received.
    #[error("Missed keep-alive")]
    MissedKeepAlive,

    #[error("invalid verification token received")]
    InvalidVerifyToken,

    /// No target was found for the user.
    #[error("No target was found for the user")]
    NoTargetFound,

    /// The received packets is of an invalid length that we cannot process.
    #[error("illegal packets length")]
    IllegalPacketLength,

    /// The received value index cannot be mapped to an existing enum.
    #[error("illegal enum value index for {kind}: {value}")]
    IllegalEnumValue {
        /// The enum kind which was parsed.
        kind: &'static str,
        /// The value that was received.
        value: VarInt,
    },

    /// The received packets ID is not mapped to an expected packet.
    #[error("unexpected packet id received {0}")]
    UnexpectedPacketId(VarInt),

    /// The string of a packet is incorrectly encoded (not UTF-8).
    #[error("invalid utf8 encoding: {0}")]
    Utf8(#[from] FromUtf8Error),

    /// Some array conversion failed.
    #[error("could not convert into array")]
    ArrayConversionFailed,

    /// Some fastnbt error.
    #[error("failed to parse nbt: {0}")]
    Nbt(#[from] passage_packets::fastnbt::error::Error),

    /// An error occurred during the invocation or communication of an adapter.
    #[error("failed to invoke adapter: {0}")]
    AdapterError(#[from] passage_adapters::Error),

    /// An error occurred during the packets handling (read/write).
    #[error("failed to handle packet: {0}")]
    Packets(#[from] passage_packets::Error),
}
