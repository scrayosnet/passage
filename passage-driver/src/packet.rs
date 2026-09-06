//! Packet identity, and the trait every packet implements by hand.
//!
//! A packet type is declared **once** and carries its own version mapping. Both directions of the
//! mapping are derived from that single declaration:
//!
//! * *encode*: [`Packet::id`] resolves the ID for the connection's version, so no call site ever
//!   writes a literal ID.
//! * *decode*: [`Router`](crate::router::Router) builds a `(phase, id) -> handler` table by asking
//!   every registered packet for its ID at each supported version.
//!
//! # Why these are written out by hand
//!
//! An earlier iteration generated `decode`/`encode` from a `packet!` macro. It saved about ten
//! lines per packet and cost the ability to say anything the macro had not anticipated:
//!
//! * **Per-field limits.** A generated decoder can only apply one crate-wide string limit, so a
//!   hostname field and a chat field were both allowed 98 KB. Written out, each field names its own
//!   bound.
//! * **Domain types.** Because the macro picked an encoding from a field's *type*, packets had to
//!   carry `VarInt` newtypes and handlers had to parse them (`match packet.intent.0 { 1 => .. }`).
//!   Written out, the decoder produces an [`Intent`](crate::demo::packets::Intent) and the handler
//!   cannot see an invalid one.
//! * **Version-dependent shape.** A version that reorders, splits or retypes a field is an `if`
//!   here. Under the macro it needed a second packet type, which is the outcome
//!   `docs/02-versioning.md` argues against.
//!
//! What does *not* have to be repeated per packet is the trailing-bytes check: the router calls
//! [`Reader::finish`] after every decode, so no decoder can forget it.

use crate::error::Result;
use crate::version::ProtocolVersion;
use crate::wire::{Reader, Writer};

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

/// Which way a packet travels.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Client to server.
    Serverbound,
    /// Server to client.
    Clientbound,
}

impl Direction {
    /// The opposite direction.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Direction::Serverbound => Direction::Clientbound,
            Direction::Clientbound => Direction::Serverbound,
        }
    }
}

/// A protocol packet.
pub trait Packet: Sized + Send + Sync + 'static {
    /// The name of the packet, used for tracing, metrics and error messages.
    const NAME: &'static str;

    /// The phase this packet belongs to.
    const PHASE: Phase;

    /// The direction this packet travels in.
    const DIRECTION: Direction;

    /// The ID of this packet in each version that changed it, **ordered newest to oldest**:
    ///
    /// ```ignore
    /// const IDS: &[(ProtocolVersion, i32)] =
    ///     &[(versions::V1_21_2, 0x03), (versions::V1_20_5, 0x02)];
    /// ```
    ///
    /// Data rather than a function, because the router does not only *ask* for IDs -- it reads the
    /// thresholds to work out where the protocol changes shape, and builds one dispatch table per
    /// change rather than one per version anybody remembered to list. See
    /// [`RouterBuilder::build`](crate::router::RouterBuilder::build).
    ///
    /// The order is load-bearing and checked at build time
    /// ([`BuildError::UnorderedIds`](crate::error::BuildError::UnorderedIds)): a table written
    /// oldest-first would silently resolve to the wrong ID.
    const IDS: &'static [(ProtocolVersion, i32)];

    /// Decodes the packet payload (without length prefix and ID).
    ///
    /// The caller checks that the whole payload was consumed, so a decoder does not call
    /// [`Reader::finish`] itself.
    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self>;

    /// Encodes the packet payload (without length prefix and ID).
    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()>;

    /// The packet ID in `version`, or [`None`] if the packet does not exist there.
    ///
    /// Resolved from [`IDS`](Packet::IDS); there is no reason to override it.
    #[must_use]
    fn id(version: ProtocolVersion) -> Option<i32> {
        ids(version, Self::IDS)
    }
}

/// Resolves a packet ID from a table of `(first version, id)` pairs, **ordered newest to oldest**.
///
/// The first entry whose version is `<=` the connection's version wins. If none matches, the packet
/// does not exist in that version and the result is [`None`] -- which makes sending it an internal
/// error instead of a malformed frame, and keeps it out of the decode table entirely.
///
/// A version the table cannot order -- a snapshot, a negative number -- resolves against the floor
/// instead, exactly as [`Router`](crate::router::Router) picks its dispatch table. See
/// [`ProtocolVersion::placed`] for why the two have to agree.
#[must_use]
pub fn ids(version: ProtocolVersion, table: &[(ProtocolVersion, i32)]) -> Option<i32> {
    let version = version.placed();
    table
        .iter()
        .find(|(since, _)| version.at_least(*since))
        .map(|(_, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::versions;

    #[test]
    fn ids_pick_the_newest_matching_entry() {
        let table = [(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)];
        assert_eq!(ids(versions::V26_2, &table), Some(0x05));
        assert_eq!(ids(versions::V1_21, &table), Some(0x02));
        assert_eq!(ids(versions::V1_20_5, &table), Some(0x02));
        // Below the oldest entry the packet does not exist.
        assert_eq!(ids(ProtocolVersion::new(765), &table), None);
        // A hostile version must not resolve to anything either.
        assert_eq!(ids(ProtocolVersion::new(i32::MIN), &table), None);
    }

    #[test]
    fn a_version_that_cannot_be_placed_resolves_like_the_floor() {
        let versioned = [(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)];
        let anchored = [(ProtocolVersion::UNKNOWN, 0x00)];
        // A snapshot is numerically above every release, so a plain `>=` would hand it the newest
        // ID -- while the router dispatches it against the floor table. That disagreement is the
        // bug: an ID we would accept is not one we would send.
        let snapshot = ProtocolVersion::new(0x4000_0000 | 132);
        assert_eq!(ids(snapshot, &versioned), None);
        assert_eq!(ids(snapshot, &anchored), Some(0x00));
        // And a client that sends garbage can still be answered with the packets that never
        // depended on a version: a status response, and a reason for turning it away.
        assert_eq!(ids(ProtocolVersion::new(-1), &anchored), Some(0x00));
    }

    #[test]
    fn phase_indices_are_dense_and_stable() {
        for (index, phase) in Phase::ALL.into_iter().enumerate() {
            assert_eq!(phase.index(), index);
        }
    }
}
