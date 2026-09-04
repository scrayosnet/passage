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
//!
//! Wiring mistakes are not in here at all. They are [`BuildError`]s, produced once by
//! [`RouterBuilder::build`](crate::router::RouterBuilder::build) at startup, and a connection can
//! never encounter one.

use crate::packet::{Direction, Phase};
use crate::version::ProtocolVersion;

/// Who caused an error. This drives the observability and disconnect policy.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Class {
    /// The peer sent something illegal, or stopped playing along. Expected in the wild (scanners,
    /// mods, bots, timeouts): count it, log it at `debug`, never page anyone. Do not report to
    /// error tracking.
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

    /// A value was not one of the variants the protocol defines for its field.
    #[error("invalid value {value} for field `{field}`")]
    InvalidValue {
        /// The field being read.
        field: &'static str,
        /// The value that was read.
        value: i32,
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

    /// A packet arrived that is registered, but in a different phase than the connection is in.
    #[error("packet `{packet}` belongs to phase {expected:?} but arrived in phase {phase:?}")]
    UnexpectedPacket {
        /// The packet the ID resolves to in some other phase.
        packet: &'static str,
        /// The phase that packet belongs to.
        expected: Phase,
        /// The phase the connection was in.
        phase: Phase,
    },

    /// A packet arrived while the peer was required to stay quiet.
    ///
    /// A connection gates reads while an exclusive handler task is in flight (an authentication call,
    /// a session-server round trip). A well-behaved peer waits for the answer, so a frame arriving
    /// in that window was sent too early -- which is a protocol break, not backpressure.
    #[error("packet id {id:#04x} arrived in phase {phase:?} while the peer had to wait")]
    EarlyPacket {
        /// The phase the connection was in.
        phase: Phase,
        /// The ID of the packet that arrived too early.
        id: i32,
    },

    /// The peer's protocol version is not supported.
    #[error("unsupported protocol version {version}")]
    UnsupportedVersion {
        /// The version the peer announced.
        version: ProtocolVersion,
    },
}

/// An error caused by us: a bug in the layer above the driver.
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

    /// A queued packet was encoded for a configuration the connection had left by the time the
    /// operation was drained.
    ///
    /// A handler sees a *snapshot* of the version and phase, and encodes against it. That is only
    /// wrong if the same handler also moved the connection first -- `set_phase(Configuration)`
    /// followed by a send of a `Login` packet, or a send after `set_version`. Operations drain in
    /// queue order, so the correct orderings (send, *then* switch) can never trip this; only the
    /// mistake can.
    ///
    /// Without the check those bytes would go out with an ID the peer resolves against a different
    /// table, which is a desynchronised connection with no diagnostic on either side.
    #[error(
        "`{packet}` was encoded for version {encoded_version} phase {encoded_phase:?}, but the \
         connection reached version {version} phase {phase:?} before it was written"
    )]
    StaleEncoding {
        /// The packet that was queued.
        packet: &'static str,
        /// The version it was encoded for.
        encoded_version: ProtocolVersion,
        /// The phase it belongs to.
        encoded_phase: Phase,
        /// The version the connection is in now.
        version: ProtocolVersion,
        /// The phase the connection is in now.
        phase: Phase,
    },
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

    /// Raised by a handler.
    ///
    /// The driver cannot know whether a failed authentication is the peer's fault, ours or a
    /// dependency's, so the handler supplies both the blame and the metric label. Without this the
    /// only channel a handler had was an internal error, which meant every ordinary rejection --
    /// a missed keep-alive, an unverified profile -- was logged at `warn` and reported.
    #[error("{source}")]
    Handler {
        /// Who is to blame.
        class: Class,
        /// A stable, low-cardinality metric label. Never peer-controlled.
        label: &'static str,
        /// The underlying error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The connection is already gone, so the operation could not be carried out. This is not a
    /// failure of the connection -- it *is* the connection ending -- and callers usually ignore it.
    #[error("the connection is closed")]
    Closed,
}

impl Error {
    /// Raises a handler error the peer is to blame for: a rejection, a timeout, a failed check.
    #[must_use]
    pub fn peer(
        label: &'static str,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error::Handler {
            class: Class::Peer,
            label,
            source: source.into(),
        }
    }

    /// Raises a handler error we are to blame for: a bug, a misconfiguration, a failing dependency.
    #[must_use]
    pub fn internal(
        label: &'static str,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Error::Handler {
            class: Class::Internal,
            label,
            source: source.into(),
        }
    }

    /// Who is to blame for this error.
    #[must_use]
    pub fn class(&self) -> Class {
        match self {
            Error::Protocol(_) => Class::Peer,
            Error::Transport(_) | Error::Closed => Class::Transport,
            Error::Internal(_) => Class::Internal,
            Error::Handler { class, .. } => *class,
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
                ProtocolError::InvalidValue { .. } => "invalid_value",
                ProtocolError::FrameTooLarge { .. } => "frame_too_large",
                ProtocolError::UnknownPacket { .. } => "unknown_packet",
                ProtocolError::UnexpectedPacket { .. } => "unexpected_packet",
                ProtocolError::EarlyPacket { .. } => "early_packet",
                ProtocolError::UnsupportedVersion { .. } => "unsupported_version",
            },
            Error::Transport(_) => "transport",
            Error::Internal(_) => "internal",
            Error::Handler { label, .. } => label,
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

/// A mistake in how the router was assembled.
///
/// These are separate from [`Error`] on purpose. A build error is a wiring bug: it does not depend
/// on any input, it is the same on every run, and it is discovered once --
/// [`RouterBuilder::build`](crate::router::RouterBuilder::build) either produces a router that
/// works for every supported version or it fails at startup. Keeping them out of [`Error`] is why
/// [`Connection::new`](crate::conn::Connection::new) cannot fail.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    /// A packet was registered on a router that does not receive packets travelling its way.
    #[error("packet `{packet}` travels {actual:?} but this router receives {expected:?} packets")]
    WrongDirection {
        /// The packet that was registered.
        packet: &'static str,
        /// The direction the packet travels in.
        actual: Direction,
        /// The direction the router receives.
        expected: Direction,
    },

    /// A packet's ID is outside the range the dispatch table covers.
    #[error("packet `{packet}` has out-of-range id {id} in version {version}")]
    IdOutOfRange {
        /// The packet that was registered.
        packet: &'static str,
        /// The offending ID.
        id: i32,
        /// The version the ID was resolved for.
        version: ProtocolVersion,
    },

    /// Two packets resolve to the same ID in the same phase and version.
    #[error(
        "packets `{first}` and `{second}` both claim id {id:#04x} in phase {phase:?} of version \
         {version}"
    )]
    IdCollision {
        /// The packet that claimed the ID first.
        first: &'static str,
        /// The packet that collided with it.
        second: &'static str,
        /// The contested ID.
        id: i32,
        /// The phase both packets belong to.
        phase: Phase,
        /// The version the IDs were resolved for.
        version: ProtocolVersion,
    },

    /// More packets were registered than the dispatch table can index.
    #[error("{count} packets registered, but the dispatch table indexes at most {limit}")]
    TooManyPackets {
        /// The number of registered packets.
        count: usize,
        /// The maximum the table can index.
        limit: usize,
    },
}

/// The driver's result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;
