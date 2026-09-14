use thiserror::Error;
use crate::codec::CodecError;
use crate::phase::Phase;
use crate::version::ProtocolVersion;

pub type Result<T> = std::result::Result<T, ConnectionError>;

#[derive(Debug, Error)]
pub enum ConnectionError {
    /// The codec (io) raised an error.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// The connection is closed.
    #[error("the connection is closed")]
    OpsClosed, // TODO maybe introduce dispatch error?

    /// The peer hung up.
    ///
    /// Nothing can be sent after this. It is still an ending rather than a completion because the
    /// peer leaving is not us being finished with it -- a client that disappears while its backend
    /// is being selected has left a selection running.
    #[error("the peer hung up")]
    PeerClosed,

    /// The shutdown token was cancelled.
    #[error("the connection was cancelled")]
    Cancelled,

    /// A deadline expired.
    #[error("the connection ran out of time")]
    TimedOut,

    /// A queued packet was encoded for a protocol version the connection had left by the time the
    /// operation was drained.
    ///
    /// A handler sees a *snapshot* of the version and encodes against it. That is only wrong if the
    /// same handler also moved the connection first -- a send after `set_version`. Operations drain
    /// in queue order, so the correct ordering (send, *then* switch) can never trip this; only the
    /// mistake can.
    ///
    /// Without the check those bytes would go out with an ID the peer resolves against a different
    /// table, which is a desynchronised connection with no diagnostic on either side.
    ///
    /// There is no phase counterpart, and deliberately so: a packet belongs to exactly one phase
    /// ([`Packet::PHASE`](crate::packet::Packet::PHASE)), so "the phase it was encoded for" was
    /// never a snapshot of anything -- it was the packet's own identity, and checking a constant
    /// against the connection only ever restated what the type already said.
    #[error(
        "`{packet}` was encoded for version {encoded_version}, but the connection reached version \
         {version} before it was written"
    )]
    StaleEncoding {
        /// The packet that was queued.
        packet: &'static str,
        /// The version it was encoded for.
        encoded_version: ProtocolVersion,
        /// The version the connection is in now.
        version: ProtocolVersion,
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
}
