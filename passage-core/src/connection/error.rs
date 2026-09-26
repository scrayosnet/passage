use crate::codec::CodecError;
use crate::common::Phase;
use thiserror::Error;

/// The connection result type, defaulting to [`ConnectionError`].
pub type Result<T, E = ConnectionError> = std::result::Result<T, E>;

/// The reason for why the connection was closed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CloseReason {
    /// The peer closed the connection.
    Peer,

    /// The connection timed out.
    Timeout,

    /// The connection was shut down (or unknown reason). This will only happen after the shutdown
    /// token has already been canceled for any reason.
    #[default]
    Shutdown,
}

/// The connection module error type. It handles all errors that can occur during the connection.
#[derive(Debug, Error)]
pub enum ConnectionError {
    /// The codec (io) raised an error.
    #[error(transparent)]
    Codec(#[from] CodecError),

    /// The connection is closed for some reason.
    #[error("the connection is closed: {reason:?}")]
    Closed {
        /// The reason for why the connection was closed. Defaults to [`CloseReason::Shutdown`]
        reason: CloseReason,
    },

    /// A packet arrived while the peer was required to stay quiet: see
    /// [`Conn::gate`](crate::connection::Conn::gate).
    #[error("packet id {id:#04x} arrived in phase {phase:?} while the peer had to wait")]
    EarlyPacket {
        /// The phase the connection was in.
        phase: Phase,

        /// The ID of the packet that arrived too early.
        id: i32,
    },
}

impl ConnectionError {
    /// Creates a new closed error because the connection was shut down.
    pub fn shutdown() -> Self {
        Self::Closed {
            reason: CloseReason::Shutdown,
        }
    }

    /// Creates a new closed error because the peer closed the connection.
    pub fn peer() -> Self {
        Self::Closed {
            reason: CloseReason::Peer,
        }
    }

    /// Creates a new closed error because the connection timed out.
    pub fn timeout() -> Self {
        Self::Closed {
            reason: CloseReason::Timeout,
        }
    }

    /// Gets the reason key for the error. This is a stable, low-cardinality label for logs, spans
    /// and metrics, and never contains peer-controlled data.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            ConnectionError::Codec(_) => "codec",
            ConnectionError::Closed {
                reason: CloseReason::Peer,
            } => "peer-closed",
            ConnectionError::Closed {
                reason: CloseReason::Shutdown,
            } => "shutdown",
            ConnectionError::Closed {
                reason: CloseReason::Timeout,
            } => "peer-timeout",
            ConnectionError::EarlyPacket { .. } => "early-packet",
        }
    }

    /// Gets whether the error was raised because of the peer.
    ///
    /// This decides the log level: a peer error is ordinary weather and belongs at `debug`, while
    /// everything else is ours and an operator has to see it.
    #[must_use]
    pub fn is_peer_error(&self) -> bool {
        match self {
            // A malformed frame is the peer's doing; a broken socket is nobody's.
            ConnectionError::Codec(CodecError::Wire(_)) => true,
            ConnectionError::Codec(_) => false,
            ConnectionError::Closed {
                reason: CloseReason::Peer | CloseReason::Timeout,
            } => true,
            ConnectionError::Closed {
                reason: CloseReason::Shutdown,
            } => false,
            ConnectionError::EarlyPacket { .. } => true,
        }
    }

    /// Whether anything can still be written to the peer.
    ///
    /// Only a hangup answers this for certain. A broken transport or a peer that stopped reading
    /// will refuse the write too, but there is no way to know that without trying -- so this is the
    /// one case worth checking before composing a message nobody will read.
    #[must_use]
    pub fn can_reply(&self) -> bool {
        !matches!(
            self,
            ConnectionError::Closed {
                reason: CloseReason::Peer
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_error() -> Vec<ConnectionError> {
        vec![
            ConnectionError::Codec(crate::wire::WireError::Utf8 { field: "host" }.into()),
            ConnectionError::Codec(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
            ConnectionError::shutdown(),
            ConnectionError::peer(),
            ConnectionError::timeout(),
            ConnectionError::EarlyPacket {
                phase: Phase::Login,
                id: 0x03,
            },
        ]
    }

    #[test]
    fn every_reason_is_a_stable_low_cardinality_label() {
        // Logs, spans and metrics are keyed by this, so it may never carry peer-controlled data.
        let reasons: Vec<_> = every_error().iter().map(ConnectionError::reason).collect();
        assert_eq!(
            reasons,
            vec![
                "codec",
                "codec",
                "shutdown",
                "peer-closed",
                "peer-timeout",
                "early-packet",
            ],
        );
    }

    #[test]
    fn blame_decides_the_log_level_and_is_never_guessed() {
        // A malformed frame is the peer's doing; a broken socket is nobody's, and an unclassified
        // handler failure is ours -- because assuming the peer's fault would hide our own bugs.
        let blame: Vec<_> = every_error()
            .iter()
            .map(ConnectionError::is_peer_error)
            .collect();
        assert_eq!(blame, vec![true, false, false, true, true, true]);
    }

    #[test]
    fn only_a_hangup_answers_whether_anything_can_still_be_written() {
        // A broken transport or a peer that stopped reading will refuse the write too, but there is
        // no way to know that without trying. A hangup is the one case worth checking first.
        assert!(!ConnectionError::peer().can_reply());
        assert!(ConnectionError::timeout().can_reply());
        assert!(ConnectionError::shutdown().can_reply());
    }

    #[test]
    fn a_close_reason_reads_the_same_in_the_message_as_in_the_label() {
        assert_eq!(CloseReason::default(), CloseReason::Shutdown);
        assert!(
            ConnectionError::timeout().to_string().contains("Timeout"),
            "{}",
            ConnectionError::timeout(),
        );
    }
}
