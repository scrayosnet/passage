use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::{ConnRef, DispatchError};
use bytes::Bytes;
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::warn;

/// The dispatch handler type with an erased packet type.
pub type ErasedHandler<S> = Box<
    dyn for<'a> Fn(ConnRef<'a, S>, Bytes) -> BoxFuture<'a, Result<(), DispatchError>> + Send + Sync,
>;

/// The handler type for the open hook, which has no packet to hand over.
pub type Hook<S> =
    Arc<dyn for<'a> Fn(ConnRef<'a, S>) -> BoxFuture<'a, Result<(), DispatchError>> + Send + Sync>;

/// A packet handler entry. It contains the packet meta and dispatch handler.
pub struct Entry<S> {
    /// The packet name used for tracing.
    pub(crate) name: &'static str,

    /// The packet phase used to get the dispatch table.
    pub(crate) phase: Phase,

    /// The packet declared ID table (ordered).
    pub(crate) ids: &'static [(ProtocolVersion, i32)],

    /// The packet dispatch handler.
    pub(crate) dispatch: ErasedHandler<S>,
}

/// The highest packet ID a registration may claim.
///
/// This is a sanity ceiling, not the table's width: a table is sized by the IDs actually registered
/// in its phase. It exists because the slot is the index, so an absurd ID would size the table
/// rather than be rejected by it. It sits above the Play phase, whose client-bound IDs already run
/// past `0x80`.
const MAX_PACKET_ID: i32 = 1023;

/// The dispatch table for one interval of protocol versions.
pub struct Table {
    /// The phase table. It has one id map per phase. The map is represented as a list of optional
    /// indices into the router dispatch handler (i.e., [`Entry`]).
    pub(crate) by_phase: [Box<[Option<u16>]>; Phase::COUNT],
}

impl Table {
    /// Finds the protocol versions that represent breakpoints for the provided entries in ascending
    /// order (to allow binary searches). Breakpoints define thresholds at which the dispatch changes,
    /// plus the floor.
    pub fn breakpoints<S>(entries: &[Entry<S>]) -> Vec<ProtocolVersion> {
        // The floor is always present: it is what a pre-handshake connection, and anything the version
        // table cannot place, dispatches against.
        let mut versions = vec![ProtocolVersion::UNKNOWN];
        versions.extend(
            entries
                .iter()
                .flat_map(|entry| entry.ids.iter().map(|(since, _)| *since)),
        );
        versions.sort_unstable();
        versions.dedup();
        versions
    }

    /// Creates a new [`Table`] from the entries and protocol version. On conflict, the previous entry
    /// is overwritten in the table (printing a warning). This may, but should not, be used to overwrite
    /// from default configurations.
    ///
    /// A packet whose ID falls outside `0..=MAX_PACKET_ID` is skipped with a warning rather than
    /// registered.
    pub fn new<S>(entries: &[Entry<S>], version: ProtocolVersion) -> Self {
        let mut by_phase: [Vec<Option<u16>>; Phase::COUNT] = Default::default();
        for (index, entry) in entries.iter().enumerate() {
            let Some(id) = crate::packet::packet::ids(version, entry.ids) else {
                continue;
            };

            if !(0..=MAX_PACKET_ID).contains(&id) {
                warn!(
                    packet = entry.name,
                    packet_id = id,
                    limit = MAX_PACKET_ID,
                    version = %version,
                    "Packet ID out of range, skipping registration"
                );
                continue;
            }

            let table = &mut by_phase[entry.phase.index()];
            let slot = id as usize;
            if table.len() <= slot {
                table.resize(slot + 1, None);
            }
            if let Some(existing) = table[slot] {
                warn!(
                    previous = entries[existing as usize].name,
                    current = entry.name,
                    packet_id = id,
                    phase = ?entry.phase,
                    version = %version,
                    "Duplicate packet ID detected, overwriting previous"
                );
            }
            // Bounded by `MAX_PACKETS`, checked before this runs.
            table[slot] = Some(index as u16);
        }

        Self {
            by_phase: by_phase.map(Vec::into_boxed_slice),
        }
    }

    /// Gets the dispatch handler for the given packet ID and phase. Returns `None` if no handler is
    /// registered.
    pub fn lookup(&self, phase: Phase, id: i32) -> Option<u16> {
        let slot = usize::try_from(id).ok()?;
        *self.by_phase[phase.index()].get(slot)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::versions;

    fn entry<S>(
        name: &'static str,
        phase: Phase,
        ids: &'static [(ProtocolVersion, i32)],
    ) -> Entry<S> {
        Entry {
            name,
            phase,
            ids,
            dispatch: Box::new(|_, _| Box::pin(std::future::ready(Ok(())))),
        }
    }

    #[test]
    fn breakpoints_are_the_versions_the_packets_name() {
        // Which versions get a table is not something the caller has to know: the packets say.
        let entries: Vec<Entry<()>> = vec![
            entry(
                "Newer",
                Phase::Login,
                &[(versions::V26_1, 0x05), (versions::V1_20_5, 0x02)],
            ),
            entry("Older", Phase::Login, &[(versions::V1_20_5, 0x03)]),
            entry(
                "Anchored",
                Phase::Handshake,
                &[(ProtocolVersion::UNKNOWN, 0x00)],
            ),
        ];
        assert_eq!(
            Table::breakpoints(&entries),
            vec![ProtocolVersion::UNKNOWN, versions::V1_20_5, versions::V26_1],
            "ascending, deduplicated, and the floor is always present",
        );
    }

    #[test]
    fn the_floor_is_present_even_when_no_packet_names_it() {
        // It is what a pre-handshake connection, and anything the version table cannot place,
        // dispatches against.
        let entries: Vec<Entry<()>> = vec![entry("Only", Phase::Login, &[(versions::V26_1, 0x01)])];
        assert_eq!(
            Table::breakpoints(&entries),
            vec![ProtocolVersion::UNKNOWN, versions::V26_1],
        );
        assert_eq!(
            Table::breakpoints::<()>(&[]),
            vec![ProtocolVersion::UNKNOWN]
        );
    }

    #[test]
    fn a_table_indexes_by_phase_and_id() {
        let entries: Vec<Entry<()>> = vec![
            entry("First", Phase::Login, &[(ProtocolVersion::UNKNOWN, 0x00)]),
            entry("Second", Phase::Status, &[(ProtocolVersion::UNKNOWN, 0x00)]),
            entry("Third", Phase::Login, &[(ProtocolVersion::UNKNOWN, 0x04)]),
        ];
        let table = Table::new(&entries, ProtocolVersion::UNKNOWN);

        // IDs are only unique within a phase, which is why the phase is part of the key.
        assert_eq!(table.lookup(Phase::Login, 0x00), Some(0));
        assert_eq!(table.lookup(Phase::Status, 0x00), Some(1));
        assert_eq!(table.lookup(Phase::Login, 0x04), Some(2));
        // A gap inside the table, an ID past its end, and a phase nobody registered.
        assert_eq!(table.lookup(Phase::Login, 0x01), None);
        assert_eq!(table.lookup(Phase::Login, 0x99), None);
        assert_eq!(table.lookup(Phase::Configuration, 0x00), None);
    }

    #[test]
    fn a_packet_that_does_not_exist_in_a_version_is_not_in_its_table() {
        let entries: Vec<Entry<()>> =
            vec![entry("Newer", Phase::Login, &[(versions::V26_1, 0x05)])];
        assert_eq!(
            Table::new(&entries, versions::V1_20_5).lookup(Phase::Login, 0x05),
            None,
        );
        assert_eq!(
            Table::new(&entries, versions::V26_1).lookup(Phase::Login, 0x05),
            Some(0),
        );
    }

    #[test]
    fn an_id_out_of_range_is_skipped_rather_than_sizing_the_table() {
        // The slot is the index, so an absurd ID would otherwise resize a `Vec` to whatever the
        // packet claimed. It is skipped, and the packets around it still register.
        let entries: Vec<Entry<()>> = vec![
            entry(
                "Absurd",
                Phase::Login,
                &[(ProtocolVersion::UNKNOWN, i32::MAX)],
            ),
            entry("Negative", Phase::Login, &[(ProtocolVersion::UNKNOWN, -1)]),
            entry(
                "Ordinary",
                Phase::Login,
                &[(ProtocolVersion::UNKNOWN, 0x02)],
            ),
        ];
        let table = Table::new(&entries, ProtocolVersion::UNKNOWN);

        assert_eq!(table.lookup(Phase::Login, i32::MAX), None);
        assert_eq!(table.lookup(Phase::Login, -1), None);
        assert_eq!(table.lookup(Phase::Login, 0x02), Some(2));
        assert_eq!(
            table.by_phase[Phase::Login.index()].len(),
            3,
            "the table is sized by the IDs it accepted, not by the one it refused",
        );
    }

    #[test]
    fn the_highest_id_a_registration_may_claim_is_still_accepted() {
        let entries: Vec<Entry<()>> = vec![entry(
            "Edge",
            Phase::Play,
            &[(ProtocolVersion::UNKNOWN, MAX_PACKET_ID)],
        )];
        let table = Table::new(&entries, ProtocolVersion::UNKNOWN);
        assert_eq!(table.lookup(Phase::Play, MAX_PACKET_ID), Some(0));
    }

    #[test]
    fn a_duplicate_id_overwrites_the_registration_before_it() {
        // Documented rather than rejected, so a default set of handlers can be overridden. The
        // warning is what makes an accident visible.
        let entries: Vec<Entry<()>> = vec![
            entry("First", Phase::Login, &[(ProtocolVersion::UNKNOWN, 0x00)]),
            entry("Second", Phase::Login, &[(ProtocolVersion::UNKNOWN, 0x00)]),
        ];
        let table = Table::new(&entries, ProtocolVersion::UNKNOWN);
        assert_eq!(
            table.lookup(Phase::Login, 0x00),
            Some(1),
            "the later one wins"
        );
    }
}
