use crate::common::ProtocolVersion;
use thiserror::Error;

/// A result type that can be returned from a [`Router`](crate::router::Router) handler.
pub type Result<T, E = RouterError> = std::result::Result<T, E>;

/// An error that can occur when registering a packet with a
/// [`RouterBuilder`](crate::router::RouterBuilder).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RouterError {
    /// A packet's ID table is not ordered ascending (i.e., newest to oldest).
    #[error("packet `{packet}` lists version {version} after {previous}")]
    UnorderedIds {
        /// The packet that was registered.
        packet: &'static str,

        /// The entry that came first.
        previous: ProtocolVersion,

        /// The entry that should have come before it.
        version: ProtocolVersion,
    },

    /// More packets were registered than the dispatch table can index.
    #[error("{count} packets registered exceed limit {limit}")]
    TooManyPackets {
        /// The number of registered packets.
        count: usize,

        /// The maximum the table can index.
        limit: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::versions;

    #[test]
    fn an_unordered_table_names_the_pair_that_is_the_wrong_way_round() {
        // The table is read newest-first, so the message has to show which two entries swapped --
        // the packet name alone does not say where to look in a long table.
        let error = RouterError::UnorderedIds {
            packet: "LoginStart",
            previous: versions::V1_20_5,
            version: versions::V26_3,
        };
        let message = error.to_string();
        assert!(message.contains("LoginStart"), "{message}");
        assert!(
            message.contains("766") && message.contains("777"),
            "{message}"
        );
    }

    #[test]
    fn a_saturated_router_says_what_it_can_hold() {
        let error = RouterError::TooManyPackets {
            count: 65_536,
            limit: 65_535,
        };
        assert!(error.to_string().contains("65535"), "{error}");
    }

    #[test]
    fn a_registration_failure_can_be_compared() {
        // Which is what lets a caller match on one without reaching for a string.
        assert_eq!(
            RouterError::TooManyPackets { count: 1, limit: 0 },
            RouterError::TooManyPackets { count: 1, limit: 0 },
        );
    }
}
