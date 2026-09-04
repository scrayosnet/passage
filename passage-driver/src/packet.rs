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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Before the handshake has been processed.
    Handshake,
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
    /// The number of phases, for table sizing.
    pub const COUNT: usize = 5;

    /// Every phase, in [`Phase::index`] order.
    pub const ALL: [Phase; Self::COUNT] = [
        Phase::Handshake,
        Phase::Status,
        Phase::Login,
        Phase::Configuration,
        Phase::Play,
    ];

    /// A dense index for table lookups.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Phase::Handshake => 0,
            Phase::Status => 1,
            Phase::Login => 2,
            Phase::Configuration => 3,
            Phase::Play => 4,
        }
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

    /// The packet ID in the given protocol version, or [`None`] if the packet does not exist there.
    ///
    /// Implement this with [`ids`], which keeps the version table declarative:
    ///
    /// ```ignore
    /// fn id(version: ProtocolVersion) -> Option<i32> {
    ///     ids(version, &[(versions::V1_21_2, 0x03), (versions::V1_20_5, 0x02)])
    /// }
    /// ```
    fn id(version: ProtocolVersion) -> Option<i32>;

    /// Decodes the packet payload (without length prefix and ID).
    ///
    /// The caller checks that the whole payload was consumed, so a decoder does not call
    /// [`Reader::finish`] itself.
    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self>;

    /// Encodes the packet payload (without length prefix and ID).
    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()>;
}

/// Resolves a packet ID from a table of `(first version, id)` pairs, **ordered newest to oldest**.
///
/// The first entry whose version is `<=` the connection's version wins. If none matches, the packet
/// does not exist in that version and the result is [`None`] -- which makes sending it an internal
/// error instead of a malformed frame, and keeps it out of the decode table entirely.
#[must_use]
pub fn ids(version: ProtocolVersion, table: &[(ProtocolVersion, i32)]) -> Option<i32> {
    table
        .iter()
        .find(|(since, _)| version.at_least(*since))
        .map(|(_, id)| *id)
}

/// A packet whose type has been erased, so the driver can carry it in an
/// [`Op::Send`](crate::conn::Op::Send).
///
/// Encoding happens on the driver, at the moment the operation is drained. That is what lets a
/// handler queue a packet without knowing the negotiated version, and what keeps the bytes on the
/// wire in operation order rather than in whatever order handlers finished encoding.
pub trait AnyPacket: Send + Sync {
    /// The name of the packet.
    fn name(&self) -> &'static str;

    /// The packet ID in the given protocol version.
    fn id(&self, version: ProtocolVersion) -> Option<i32>;

    /// Encodes the payload.
    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()>;
}

/// Wraps a [`Packet`] to erase its type.
///
/// A wrapper rather than a blanket `impl<P: Packet> AnyPacket for P`, so that a packet type never
/// has two inherent-looking `encode` methods to disambiguate between.
pub(crate) struct Erased<P>(pub(crate) P);

impl<P: Packet> AnyPacket for Erased<P> {
    fn name(&self) -> &'static str {
        P::NAME
    }

    fn id(&self, version: ProtocolVersion) -> Option<i32> {
        P::id(version)
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()> {
        Packet::encode(&self.0, w, version)
    }
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
    fn phase_indices_are_dense_and_stable() {
        for (index, phase) in Phase::ALL.into_iter().enumerate() {
            assert_eq!(phase.index(), index);
        }
    }
}
