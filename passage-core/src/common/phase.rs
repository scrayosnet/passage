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
}
