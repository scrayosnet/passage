use thiserror::Error;
use crate::packet::Phase;
use crate::version::ProtocolVersion;

/// A result type that can be returned from a [`Router`] handler.
pub type Result<T, E = RouterError> = std::result::Result<T, E>;

/// An error that can occur when building a [`Router`].
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
    #[error("packets `{first}` and `{second}` conflict id {id:#04x} for phase {phase:?} and version {version}")]
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
    #[error("{count} packets registered exceed limit {limit}")]
    TooManyPackets {
        /// The number of registered packets.
        count: usize,

        /// The maximum the table can index.
        limit: usize,
    },
}
