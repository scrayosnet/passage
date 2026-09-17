use thiserror::Error;
use crate::packet::Phase;
use crate::version::ProtocolVersion;

pub type Result<T> = std::result::Result<T, RouterError>;

#[derive(Debug, Error)]
pub enum RouterError {
}

/// A mistake in how the router was assembled.
///
/// These are separate from [`Error`] on purpose. A build error is a wiring bug: it does not depend
/// on any input, it is the same on every run, and it is discovered once --
/// [`RouterBuilder::build`](crate::router::RouterBuilder::build) either produces a router that
/// works for every supported version or it fails at startup. Keeping them out of [`Error`] is why
/// [`ConnectionBuilder::build`](crate::conn::ConnectionBuilder::build) cannot fail.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    /// A packet's ID table is not ordered newest to oldest.
    ///
    /// [`ids`](crate::packet::ids) takes the first entry that matches, and the router reads the
    /// same table to work out where the protocol changes shape. Both are wrong for a table written
    /// the other way round, and neither would say so at runtime -- the packet would simply resolve
    /// to an ID from the wrong era.
    #[error("packet `{packet}` lists version {version} after {previous}; ids go newest to oldest")]
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

#[derive(Debug, Error)]
#[error("")]
pub struct HandlerError {
    /// Who is to blame.
    class: Class,

    /// A stable, low-cardinality metric label. Never peer-controlled.
    label: &'static str,

    /// The underlying error.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}
