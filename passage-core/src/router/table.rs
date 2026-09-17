use std::sync::Arc;
use tracing::warn;
use crate::connection::{ConnectionError, Ctx, DispatchError};
use crate::phase::Phase;
use crate::version::ProtocolVersion;

// TODO add additional handler error type!

/// A decoder and handler pair with the packet type erased.
///
/// A handler is synchronous by construction: everything it wants to happen it queues as an
/// [`Op`](crate::conn::Op), and anything it has to wait for it hands to
/// [`Ctx::spawn`](crate::conn::Ctx::spawn) or [`Ctx::exclusive`](crate::conn::Ctx::exclusive).
///
/// These three aliases used to be three public traits, each with a single `call` method and a
/// blanket impl over the corresponding `Fn` -- the same construction written out three times, for a
/// capability nothing used: a handler is a closure or an `fn` item. Written as the function types
/// they always were, the erasure is visible where it happens and `.on::<P, _>(f)` loses the `_` that
/// only ever stood for the handler's own type.
pub type ErasedHandler<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<(), DispatchError> + Send + Sync>;

/// The handler that runs on every tick instead of on a packet. Shared, so a dispatcher can hold the
/// router rather than a copy of it.
pub type TickHandler<S> = Arc<dyn for<'c> Fn(Ctx<'c, S>) -> Result<(), DispatchError> + Send + Sync>;

/// The handler that runs when a connection ends for a reason nobody asked for.
///
/// See [`Dispatcher::on_error`] for what it may do and what it is called for.
pub type ErrorHandler<S> = Arc<dyn for<'c> Fn(Ctx<'c, S>, &ConnectionError) -> Result<(), DispatchError> + Send + Sync>;

pub struct Entry<S> {
    pub(crate) name: &'static str,
    pub(crate) phase: Phase,
    /// The packet's own ID table, newest first. Read both to resolve an ID and to find the
    /// versions at which dispatch changes.
    pub(crate) ids: &'static [(ProtocolVersion, i32)],
    pub(crate) dispatch: ErasedHandler<S>,
}

/// The dispatch table for one interval of protocol versions: `phase -> id -> index into the
/// router's entries`.
///
/// Indices rather than pointers, so a lookup is two loads with no reference count to touch, and so
/// the table itself is free of `S` and can be shared as-is.
pub struct Table {
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
    pub fn new<S>(entries: &[Entry<S>], version: ProtocolVersion) -> Self {
        let mut by_phase: [Vec<Option<u16>>; Phase::COUNT] = Default::default();
        for (index, entry) in entries.iter().enumerate() {
            let Some(id) = crate::packet::ids(version, entry.ids) else {
                continue;
            };

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

        Self { by_phase: by_phase.map(Vec::into_boxed_slice) }
    }

    /// Gets the
    pub fn lookup(&self, phase: Phase, id: i32) -> Option<u16> {
        let slot = usize::try_from(id).ok()?;
        *self.by_phase[phase.index()].get(slot)?
    }
}
