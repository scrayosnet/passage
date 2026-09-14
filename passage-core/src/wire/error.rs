use thiserror::Error;

pub type Result<T> = std::result::Result<T, WireError>;

#[derive(Debug, Error, PartialOrd, PartialEq)]
pub enum WireError {
    /// The buffer ended in the middle of a value.
    #[error(
        "unexpected end of packet for field `{field}`: needed {needed} more byte(s), {remaining} remaining"
    )]
    Eof {
        /// The field being read.
        field: &'static str,
        /// The number of bytes the reader needed.
        needed: usize,
        /// The number of bytes that were left.
        remaining: usize,
    },

    /// A `VarInt`/`VarLong` did not terminate within its maximum number of bytes.
    #[error("{kind} exceeds its maximum encoded length for field `{field}`")]
    VarIntTooLong {
        /// The field being read.
        field: &'static str,
        /// Either `VarInt` or `VarLong`.
        kind: &'static str,
    },

    /// A `VarInt`/`VarLong` used more bytes than necessary. Accepting these allows the same value
    /// to be encoded in multiple ways, which is a classic source of parser-differential bugs.
    #[error("{kind} is not canonically encoded for field `{field}`")]
    VarIntNotCanonical {
        /// The field being read.
        field: &'static str,
        /// Either `VarInt` or `VarLong`.
        kind: &'static str,
    },

    /// A length prefix was negative.
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

    /// A string was not valid UTF-8.
    #[error("field `{field}` is not valid UTF-8")]
    Utf8 {
        /// The field being read.
        field: &'static str,
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
}
