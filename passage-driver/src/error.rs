//! The error taxonomy of the driver.
//!
//! The central idea is that an error carries *who is to blame*, because that decides everything the
//! caller wants to do with it: log level, metric, whether to report it to Sentry, and whether the
//! peer deserves a disconnect message. See [`Class`].
//!
//! Equally important is what is *not* an error: a completed connection. The driver reports normal
//! completion (including "we sent a disconnect packet on purpose") as `Ok`, never as
//! `Err(ConnectionClosed)`. Using an error variant for the happy path is how the previous
//! implementation ended up with `Ok(()) | Err(Error::ConnectionClosed)` being treated identically
//! at every call site -- one forgotten match arm away from logging normal traffic as a failure.

use crate::packet::{Direction, Phase};
use crate::version::ProtocolVersion;

/// Who caused an error. This drives the observability and disconnect policy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// The peer sent something illegal. Expected in the wild (scanners, mods, bots): count it,
    /// log it at `debug`, never page anyone. Do not report to error tracking.
    Peer,

    /// The connection broke. Usually not actionable, log at `debug`.
    Transport,

    /// A bug on our side, or a dependency failing. Log at `warn`/`error` and report it.
    Internal,
}

/// An error caused by the peer's input: everything reachable by sending bytes.
///
/// Variants are intentionally fine-grained. They are cheap (no allocation, `&'static str` labels)
/// and they let metrics distinguish "old client" from "someone is fuzzing us".
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    /// The buffer ended in the middle of a value.
    #[error("unexpected end of packet: needed {needed} more byte(s), {remaining} remaining")]
    Eof {
        /// The number of bytes the reader needed.
        needed: usize,
        /// The number of bytes that were left.
        remaining: usize,
    },

    /// A `VarInt`/`VarLong` did not terminate within its maximum number of bytes.
    #[error("{kind} exceeds its maximum encoded length")]
    VarIntTooLong {
        /// Either `VarInt` or `VarLong`.
        kind: &'static str,
    },

    /// A `VarInt`/`VarLong` used more bytes than necessary. Accepting these allows the same value
    /// to be encoded in multiple ways, which is a classic source of parser-differential bugs.
    #[error("{kind} is not canonically encoded")]
    VarIntNotCanonical {
        /// Either `VarInt` or `VarLong`.
        kind: &'static str,
    },

    /// A length prefix was negative. Casting this to `usize` is what turns a 5 byte packet into a
    /// `vec![0; 18446744073709551615]`.
    #[error("negative length {value} for field `{field}`")]
    NegativeLength {
        /// The field being read.
        field: &'static str,
        /// The decoded length.
        value: i32,
    },

    /// A length prefix exceeded the configured limit for that field.
    #[error("length {actual} for field `{field}` exceeds limit of {limit}")]
    LengthLimit {
        /// The field being read.
        field: &'static str,
        /// The configured limit.
        limit: usize,
        /// The decoded length.
        actual: usize,
    },

    /// The packet was longer than its fields. Either we are misreading it or the peer is trying to
    /// smuggle data past us; both are worth failing on.
    #[error("{remaining} trailing byte(s) after decoding `{packet}`")]
    TrailingBytes {
        /// The packet being decoded.
        packet: &'static str,
        /// The number of undecoded bytes.
        remaining: usize,
    },

    /// A string was not valid UTF-8.
    #[error("field `{field}` is not valid UTF-8")]
    Utf8 {
        /// The field being read.
        field: &'static str,
    },

    /// The frame length prefix exceeded the maximum frame size.
    #[error("frame length {length} exceeds limit of {limit}")]
    FrameTooLarge {
        /// The configured limit.
        limit: usize,
        /// The announced frame length.
        length: usize,
    },

    /// No packet is registered for this ID in the current phase and version.
    #[error("unknown packet id {id:#04x} in phase {phase:?} ({direction:?}, version {version})")]
    UnknownPacket {
        /// The phase the connection was in.
        phase: Phase,
        /// The direction the packet was read in.
        direction: Direction,
        /// The protocol version of the connection.
        version: ProtocolVersion,
        /// The packet ID that could not be resolved.
        id: i32,
    },

    /// A packet arrived that is known but not acceptable right now.
    #[error("unexpected packet `{packet}` in phase {phase:?}")]
    UnexpectedPacket {
        /// The packet that arrived.
        packet: &'static str,
        /// The phase the connection was in.
        phase: Phase,
    },

    /// The peer's protocol version is not supported.
    #[error("unsupported protocol version {version}")]
    UnsupportedVersion {
        /// The version the peer announced.
        version: ProtocolVersion,
    },
}

/// An error caused by us: a bug, a misconfiguration, or a failing dependency.
#[derive(Debug, thiserror::Error)]
pub enum InternalError {
    /// A packet does not exist in the connection's protocol version, but we tried to send it.
    #[error("packet `{packet}` does not exist in protocol version {version}")]
    PacketNotInVersion {
        /// The packet we tried to send.
        packet: &'static str,
        /// The protocol version of the connection.
        version: ProtocolVersion,
    },

    /// A field that the connection's protocol version requires was not set.
    ///
    /// This is the fail-closed half of version-gated fields: the encoder refuses to emit a packet
    /// that would be truncated on the wire rather than guessing a default.
    #[error("field `{field}` of `{packet}` is required in protocol version {version} but unset")]
    MissingField {
        /// The packet being encoded.
        packet: &'static str,
        /// The field that was `None`.
        field: &'static str,
        /// The protocol version of the connection.
        version: ProtocolVersion,
    },

    /// Anything a handler wants to bubble up.
    #[error("{0}")]
    Handler(Box<dyn std::error::Error + Send + Sync>),
}

/// The driver's error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The peer misbehaved.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    /// The transport failed.
    #[error("transport error: {0}")]
    Transport(#[from] std::io::Error),

    /// We misbehaved.
    #[error(transparent)]
    Internal(#[from] InternalError),

    /// The connection is already gone, so the operation could not be carried out. This is not a
    /// failure of the connection -- it *is* the connection ending -- and callers usually ignore it.
    #[error("the connection is closed")]
    Closed,
}

impl Error {
    /// Who is to blame for this error.
    #[must_use]
    pub fn class(&self) -> Class {
        match self {
            Error::Protocol(_) => Class::Peer,
            Error::Transport(_) | Error::Closed => Class::Transport,
            Error::Internal(_) => Class::Internal,
        }
    }

    /// A stable, low-cardinality label for metrics. Never contains peer-controlled data.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Error::Protocol(err) => match err {
                ProtocolError::Eof { .. } => "eof",
                ProtocolError::VarIntTooLong { .. } => "varint_too_long",
                ProtocolError::VarIntNotCanonical { .. } => "varint_not_canonical",
                ProtocolError::NegativeLength { .. } => "negative_length",
                ProtocolError::LengthLimit { .. } => "length_limit",
                ProtocolError::TrailingBytes { .. } => "trailing_bytes",
                ProtocolError::Utf8 { .. } => "utf8",
                ProtocolError::FrameTooLarge { .. } => "frame_too_large",
                ProtocolError::UnknownPacket { .. } => "unknown_packet",
                ProtocolError::UnexpectedPacket { .. } => "unexpected_packet",
                ProtocolError::UnsupportedVersion { .. } => "unsupported_version",
            },
            Error::Transport(_) => "transport",
            Error::Internal(_) => "internal",
            Error::Closed => "closed",
        }
    }

    /// Whether the error means "the reader needs more bytes" rather than "the input is bad".
    ///
    /// The frame codec uses this to distinguish a partially received frame from a malformed one.
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        matches!(self, Error::Protocol(ProtocolError::Eof { .. }))
    }
}

/// The driver's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
