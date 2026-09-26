use crate::common::State;

/// The protocol phase a packet belongs to.
///
/// The phase is part of a packet's identity: IDs are only unique within a phase and direction.
///
/// The discriminants are the table index, so adding a phase means adding a variant and adding it to
/// [`ALL`](Phase::ALL) -- [`COUNT`](Phase::COUNT) and [`index`](Phase::index) follow on their own.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Phase {
    /// Before the handshake has been processed.
    Handshake = 0,
    /// Server list ping.
    Status,
    /// Login and encryption.
    Login,
    /// Configuration, including resource packs, cookies and the transfer packet.
    Configuration,
    /// In-game. Passage never reaches this phase, but the driver is not Passage.
    Play,
}

impl Phase {
    /// Every phase, in [`Phase::index`] order.
    pub const ALL: [Phase; 5] = [
        Phase::Handshake,
        Phase::Status,
        Phase::Login,
        Phase::Configuration,
        Phase::Play,
    ];

    /// The number of phases, for table sizing.
    pub const COUNT: usize = Self::ALL.len();

    /// A dense index for table lookups.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// The phase a handshake leads into. A transfer is a login: the client that was sent here logs in
/// again, and only what the server does with it differs.
impl From<State> for Phase {
    fn from(value: State) -> Self {
        match value {
            State::Status => Phase::Status,
            State::Login | State::Transfer => Phase::Login,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indices_are_dense_and_stable() {
        // The index *is* the table slot, so a phase whose index does not match its position in
        // `ALL` would dispatch against another phase's table.
        for (index, phase) in Phase::ALL.into_iter().enumerate() {
            assert_eq!(phase.index(), index, "{phase:?}");
        }
        assert_eq!(Phase::COUNT, Phase::ALL.len());
    }

    #[test]
    fn a_transfer_logs_in_like_anything_else() {
        // The intent decides what the server does about authentication, not which packets it
        // routes: a transferred client sends a login start like any other.
        assert_eq!(Phase::from(State::Status), Phase::Status);
        assert_eq!(Phase::from(State::Login), Phase::Login);
        assert_eq!(Phase::from(State::Transfer), Phase::Login);
    }
}
