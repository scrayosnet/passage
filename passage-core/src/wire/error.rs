use thiserror::Error;

/// The wire result type, defaulting to [`WireError`].
pub type Result<T> = std::result::Result<T, WireError>;

/// An error raised while reading or writing the Minecraft wire format.
///
/// Every variant names the field it was raised for, so a malformed packet says which of its fields
/// disagreed rather than only that one did.
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

    /// An enum value was invalid.
    #[error("{kind} field `{field}` got invalid representation")]
    IllegalEnumValue {
        /// The field being read.
        field: &'static str,
        /// The enum kind being read.
        kind: &'static str,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One of every variant, so a new one cannot be added without a message to go with it.
    fn every_error() -> Vec<WireError> {
        vec![
            WireError::Eof {
                field: "server_address",
                needed: 4,
                remaining: 3,
            },
            WireError::VarIntTooLong {
                field: "protocol_version",
                kind: "VarInt",
            },
            WireError::VarIntNotCanonical {
                field: "protocol_version",
                kind: "VarLong",
            },
            WireError::NegativeLength {
                field: "server_address",
                value: -1,
            },
            WireError::LengthLimit {
                field: "server_address",
                limit: 255,
                actual: 300,
            },
            WireError::Utf8 { field: "user_name" },
            WireError::TrailingBytes {
                packet: "Intention",
                remaining: 2,
            },
        ]
    }

    #[test]
    fn every_error_names_what_it_was_reading() {
        // A malformed packet has to say which of its fields disagreed, not only that one did. The
        // last variant names the packet instead, because by then the fields are all accounted for.
        for error in every_error() {
            let message = error.to_string();
            let named = message.contains("server_address")
                || message.contains("protocol_version")
                || message.contains("user_name")
                || message.contains("Intention");
            assert!(named, "{message}");
        }
    }

    #[test]
    fn a_var_int_error_says_which_of_the_two_it_was() {
        // `VarInt` and `VarLong` fail the same way and are bounded differently, so the message has
        // to distinguish them or the number it names looks wrong.
        let error = WireError::VarIntTooLong {
            field: "id",
            kind: "VarLong",
        };
        assert!(error.to_string().contains("VarLong"), "{error}");
    }
}
