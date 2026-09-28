//! What makes a type a packet, and the ID table that places it in the protocol's history.
//!
//! [`Packet`] is the trait every packet in the sibling modules implements. Its centrepiece is
//! [`Packet::IDS`]: a packet's ID in each version that changed it, as *data* rather than a lookup
//! function, so that the router can read the thresholds out of it and build one dispatch table per
//! change in the protocol's shape.

use crate::common::ProtocolVersion;
use crate::common::{Phase, VarInt};
use crate::wire::{Reader, WireError, Writer};

/// A protocol packet.
pub trait Packet: Sized + Send + Sync + 'static {
    /// The name of the packet, used for tracing, metrics and error messages.
    const NAME: &'static str;

    /// The phase this packet belongs to.
    const PHASE: Phase;

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
    /// ([`RouterError::UnorderedIds`](crate::router::RouterError::UnorderedIds)): a table written
    /// oldest-first would silently resolve to the wrong ID.
    const IDS: &'static [(ProtocolVersion, VarInt)];

    /// Decodes the packet payload (without length prefix).
    ///
    /// The caller checks that the whole payload was consumed, so a decoder does not call
    /// [`Reader::finish`] itself.
    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError>;

    /// Encodes the packet payload (without length prefix).
    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError>;

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

/// Finds the first out-of-order id in the packet ids definition. The packet ids should be ordered
/// by the protocol version.
#[must_use]
pub fn check_ids_unordered(
    ids: &'static [(ProtocolVersion, i32)],
) -> Option<(ProtocolVersion, ProtocolVersion)> {
    ids.windows(2)
        .find_map(|pair| (pair[0].0 <= pair[1].0).then_some((pair[0].0, pair[1].0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions;

    #[test]
    fn ids_pick_the_newest_matching_entry() {
        let table = [(versions::V26_3, 0x05), (versions::V1_20_5, 0x02)];
        assert_eq!(ids(versions::V26_3, &table), Some(0x05));
        assert_eq!(ids(ProtocolVersion::new(767), &table), Some(0x02));
        assert_eq!(ids(versions::V1_20_5, &table), Some(0x02));
        // Below the oldest entry the packet does not exist.
        assert_eq!(ids(ProtocolVersion::new(765), &table), None);
        // A hostile version must not resolve to anything either.
        assert_eq!(ids(ProtocolVersion::new(i32::MIN), &table), None);
        // An empty table is a packet that exists nowhere.
        assert_eq!(ids(versions::V26_3, &[]), None);
    }

    #[test]
    fn a_version_that_cannot_be_placed_resolves_like_the_floor() {
        let versioned = [(versions::V26_3, 0x05), (versions::V1_20_5, 0x02)];
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
    fn an_id_table_written_the_wrong_way_round_is_detected() {
        // `ids` takes the first entry that matches, so an ascending table resolves every version
        // above the second entry to an ID from the wrong era -- silently, and only for some clients.
        assert_eq!(
            check_ids_unordered(&[(versions::V1_20_5, 0x02), (versions::V26_3, 0x05)]),
            Some((versions::V1_20_5, versions::V26_3)),
        );
        assert_eq!(
            check_ids_unordered(&[(versions::V26_3, 0x05), (versions::V1_20_5, 0x02)]),
            None,
        );
        // A version listed twice is also out of order: the second entry is unreachable.
        assert_eq!(
            check_ids_unordered(&[(versions::V26_3, 0x05), (versions::V26_3, 0x02)]),
            Some((versions::V26_3, versions::V26_3)),
        );
        // Nothing to compare is nothing to complain about.
        assert_eq!(check_ids_unordered(&[]), None);
        assert_eq!(check_ids_unordered(&[(versions::V26_3, 0x05)]), None);
    }
}
