use crate::codec::CodecError;
use crate::connection::DispatchError;
use crate::phase::Phase;
use crate::version::ProtocolVersion;
use thiserror::Error;

/// The connection result type, defaulting to [`ConnectionError`].
pub type Result<T> = std::result::Result<T, ConnectionError>;

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

    /// A [`Dispatcher`](crate::connection::Dispatcher) (i.e., custom handler) raised an error. Or an error occurred while preparing
    /// the dispatch (e.g., packet parsing for types handlers).
    #[error("dispatch failed: {0}")]
    Dispatch(#[from] DispatchError),

    /// The connection is closed for some reason.
    #[error("the connection is closed: {reason:?}")]
    Closed {
        /// The reason for why the connection was closed. Defaults to [`CloseReason::Shutdown`]
        reason: CloseReason,
    },

    /// A packet was queued for the peer at a different protocol version that the connection is currently
    /// in. This might occur when an async handler sends a packet while another handler updated the
    /// version or queued a version update.
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

    /// A packet arrived while the peer was required to stay quiet (i.e., exclusive handlers).
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
            // The handler chose its own label, which is the whole point of carrying one.
            ConnectionError::Dispatch(err) => err.label,
            ConnectionError::Closed {
                reason: CloseReason::Peer,
            } => "peer-closed",
            ConnectionError::Closed {
                reason: CloseReason::Shutdown,
            } => "shutdown",
            ConnectionError::Closed {
                reason: CloseReason::Timeout,
            } => "peer-timeout",
            ConnectionError::StaleEncoding { .. } => "stale-encoding",
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
            ConnectionError::Dispatch(err) => err.is_peer_error(),
            ConnectionError::Closed {
                reason: CloseReason::Peer | CloseReason::Timeout,
            } => true,
            ConnectionError::Closed {
                reason: CloseReason::Shutdown,
            } => false,
            ConnectionError::StaleEncoding { .. } => false,
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
    use crate::connection::Class;
    use crate::version::versions;
    use anyhow::anyhow;

    fn every_error() -> Vec<ConnectionError> {
        vec![
            ConnectionError::Codec(crate::wire::WireError::Utf8 { field: "host" }.into()),
            ConnectionError::Codec(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
            ConnectionError::Dispatch(DispatchError::peer("refused", anyhow!("not today"))),
            ConnectionError::Dispatch(DispatchError::internal("broke", anyhow!("our bug"))),
            ConnectionError::shutdown(),
            ConnectionError::peer(),
            ConnectionError::timeout(),
            ConnectionError::StaleEncoding {
                packet: "StatusResponse",
                encoded_version: versions::V1_20_5,
                version: versions::V26_2,
            },
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
                "refused",
                "broke",
                "shutdown",
                "peer-closed",
                "peer-timeout",
                "stale-encoding",
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
        assert_eq!(
            blame,
            vec![true, false, true, false, false, true, true, false, true],
        );
    }

    #[test]
    fn only_a_hangup_answers_whether_anything_can_still_be_written() {
        // A broken transport or a peer that stopped reading will refuse the write too, but there is
        // no way to know that without trying. A hangup is the one case worth checking first.
        assert!(!ConnectionError::peer().can_reply());
        assert!(ConnectionError::timeout().can_reply());
        assert!(ConnectionError::shutdown().can_reply());
        assert!(
            ConnectionError::Dispatch(DispatchError::peer("refused", anyhow!("no"))).can_reply()
        );
    }

    #[test]
    fn a_handler_failure_carries_its_classification_across_the_conversion() {
        // The blame and the label are the two things telemetry needs, so neither may be flattened
        // when a dispatch failure becomes a connection failure -- or the other way round.
        let error = ConnectionError::from(DispatchError::peer("refused", anyhow!("not today")));
        assert_eq!(error.reason(), "refused");
        assert!(error.is_peer_error());

        let back = DispatchError::from(error);
        assert_eq!(back.class, Class::Peer);
        assert_eq!(back.label, "refused");

        // And an error nobody classified stays ours.
        let internal = DispatchError::from(ConnectionError::shutdown());
        assert_eq!(internal.class, Class::Internal);
        assert_eq!(internal.label, "shutdown");
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
