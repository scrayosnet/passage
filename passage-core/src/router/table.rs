use crate::connection::{ConnectionError, Ctx, DispatchError};
use crate::phase::Phase;
use crate::version::ProtocolVersion;
use std::sync::Arc;
use tracing::warn;

/// The dispatch handler type with an erased packet type.
pub type ErasedHandler<S> =
    Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<(), DispatchError> + Send + Sync>;

/// The tick handler type.
pub type TickHandler<S> =
    Arc<dyn for<'c> Fn(Ctx<'c, S>) -> Result<(), DispatchError> + Send + Sync>;

/// The error handler type.
pub type ErrorHandler<S> =
    Arc<dyn for<'c> Fn(Ctx<'c, S>, &ConnectionError) -> Result<(), DispatchError> + Send + Sync>;

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
    /// A packet whose ID falls outside `0..=`[`MAX_PACKET_ID`] is skipped with a warning rather than
    /// registered.
    pub fn new<S>(entries: &[Entry<S>], version: ProtocolVersion) -> Self {
        let mut by_phase: [Vec<Option<u16>>; Phase::COUNT] = Default::default();
        for (index, entry) in entries.iter().enumerate() {
            let Some(id) = crate::packet::ids(version, entry.ids) else {
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
