//! Typed packet registration with erased dispatch.
//!
//! Registration is generic (`.on::<LoginStart, _>(on_login_start)`), so the handler receives a
//! decoded packet and a wrong pairing does not compile. Storage is erased, so dispatch is a table
//! lookup and one virtual call -- and, crucially, adding a packet does not change any trait, which
//! means it does not break everything that already exists.
//!
//! This is the axum/tonic shape rather than the "one big trait with a method per packet" shape.
//!
//! # Tables are built once, one per *change*
//!
//! [`RouterBuilder::build`] resolves every registered packet's ID and produces the dispatch tables
//! up front. So an ID collision is a startup failure rather than a runtime error on the first
//! client that happens to send the packet, and a connection allocates nothing: it borrows an
//! [`Arc`] of the table for its version.
//!
//! Which versions get a table is not something the caller has to know. Every packet declares its
//! IDs as data ([`Packet::IDS`]), so the router can read the *thresholds* out of them -- the
//! versions at which some packet appeared, vanished or was renumbered -- and build one table per
//! threshold. Between two thresholds nothing about dispatch differs, so one table serves the whole
//! interval and a version nobody thought to enumerate is dispatched exactly like its neighbours.
//!
//! Below the lowest threshold sits the version-independent table: exactly the packets whose IDs
//! start at [`ProtocolVersion::UNKNOWN`], which is enough to answer a status ping and refuse a
//! login. Versions that are not comparable at all -- snapshots, negative numbers -- get that table
//! too, because [`at_least`](ProtocolVersion::at_least) cannot place them (see
//! [`ProtocolVersion::is_release`]).
//!
//! # How a connection reaches this
//!
//! Not directly. A [`Connection`](crate::conn::Connection) depends on the
//! [`Dispatcher`] trait, and [`RouterDispatcher`] is the implementation that dispatches to a
//! router: one per connection, holding an [`Arc`] of the shared router plus the table for the
//! version that connection negotiated. That is where the version-to-table binding lives, and it is
//! why the table type never leaves this module.

use crate::conn::{Ctx, Dispatcher, Ending};
use crate::error::{BuildError, ProtocolError, Result};
use crate::packet::{Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use std::sync::Arc;
use tracing::trace;

/// The highest packet ID the dispatch table covers.
///
/// Real IDs are far below this; anything above is a registration mistake.
const MAX_PACKET_ID: i32 = 255;

/// The most packets a router can hold, bounded by the table's index width.
const MAX_PACKETS: usize = u16::MAX as usize;

/// What to do with a packet that has no handler.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum UnknownPolicy {
    /// Fail the connection. The right default for a server that only ever needs a fixed handful of
    /// packets: anything else is either an unsupported client or someone probing.
    #[default]
    Reject,

    /// Ignore the packet. Useful for a permissive client implementation or a proxy that only cares
    /// about a few packet kinds.
    Ignore,
}

/// A handler for one packet type.
///
/// Blanket-implemented for every `fn(Ctx<'_, S>, P) -> Result<()>`, including closures. A handler
/// is synchronous by construction: everything it wants to happen it queues as an
/// [`Op`](crate::conn::Op), and anything it has to wait for it hands to
/// [`Ctx::spawn`](crate::conn::Ctx::spawn) or [`Ctx::exclusive`](crate::conn::Ctx::exclusive).
pub trait Handler<S, P>: Send + Sync + 'static {
    /// Handles one decoded packet.
    fn call(&self, ctx: Ctx<'_, S>, packet: P) -> Result<()>;
}

impl<S, P, F> Handler<S, P> for F
where
    F: Fn(Ctx<'_, S>, P) -> Result<()> + Send + Sync + 'static,
{
    fn call(&self, ctx: Ctx<'_, S>, packet: P) -> Result<()> {
        self(ctx, packet)
    }
}

/// A handler that runs on every tick instead of on a packet.
pub trait TickHandler<S>: Send + Sync + 'static {
    /// Handles one tick.
    fn call(&self, ctx: Ctx<'_, S>) -> Result<()>;
}

impl<S, F> TickHandler<S> for F
where
    F: Fn(Ctx<'_, S>) -> Result<()> + Send + Sync + 'static,
{
    fn call(&self, ctx: Ctx<'_, S>) -> Result<()> {
        self(ctx)
    }
}

/// A handler that runs when a connection ends for a reason nobody asked for.
///
/// See [`Dispatcher::on_error`] for what it may do and what it is called for.
pub trait ErrorHandler<S>: Send + Sync + 'static {
    /// Handles one ending.
    fn call(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()>;
}

impl<S, F> ErrorHandler<S> for F
where
    F: Fn(Ctx<'_, S>, &Ending) -> Result<()> + Send + Sync + 'static,
{
    fn call(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()> {
        self(ctx, ending)
    }
}

/// A decoder and handler pair with the packet type erased.
type ErasedHandler<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<()> + Send + Sync>;

struct Entry<S> {
    name: &'static str,
    phase: Phase,
    /// The packet's own ID table, newest first. Read both to resolve an ID and to find the
    /// versions at which dispatch changes.
    ids: &'static [(ProtocolVersion, i32)],
    dispatch: ErasedHandler<S>,
}

impl<S> Entry<S> {
    fn id(&self, version: ProtocolVersion) -> Option<i32> {
        crate::packet::ids(version, self.ids)
    }
}

/// The dispatch table for one interval of protocol versions: `phase -> id -> index into the
/// router's entries`.
///
/// Indices rather than pointers, so a lookup is two loads with no reference count to touch, and so
/// the table itself is free of `S` and can be shared as-is.
struct Table {
    by_phase: [Box<[Option<u16>]>; Phase::COUNT],
}

impl Table {
    fn lookup(&self, phase: Phase, id: i32) -> Option<u16> {
        let slot = usize::try_from(id).ok()?;
        *self.by_phase[phase.index()].get(slot)?
    }

    /// Finds the packet with this ID in any phase, for diagnostics only.
    fn lookup_elsewhere(&self, phase: Phase, id: i32) -> Option<u16> {
        Phase::ALL
            .into_iter()
            .filter(|other| *other != phase)
            .find_map(|other| self.lookup(other, id))
    }
}

/// Collects handlers, then produces an immutable [`Router`].
pub struct RouterBuilder<S> {
    unknown: UnknownPolicy,
    entries: Vec<Entry<S>>,
    tick: Option<Arc<dyn TickHandler<S>>>,
    on_error: Option<Arc<dyn ErrorHandler<S>>>,
    /// The first registration mistake, reported by [`RouterBuilder::build`].
    ///
    /// Deferred rather than panicked so that assembling a router is fallible in one place instead
    /// of aborting halfway through a builder chain.
    invalid: Option<BuildError>,
}

impl<S: 'static> RouterBuilder<S> {
    /// Sets what happens to packets without a handler.
    #[must_use]
    pub fn unknown(mut self, policy: UnknownPolicy) -> Self {
        self.unknown = policy;
        self
    }

    /// Registers a handler for one packet type.
    #[must_use]
    pub fn on<P, H>(mut self, handler: H) -> Self
    where
        P: Packet,
        H: Handler<S, P>,
    {
        if let Some(err) = unordered(P::NAME, P::IDS) {
            self.invalid.get_or_insert(err);
            return self;
        }

        // The trailing-bytes check lives here, so it runs for every packet and no hand-written
        // decoder has to remember it.
        let dispatch: ErasedHandler<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let version = ctx.version();
            let mut reader = Reader::new(payload, ctx.limits());
            let packet = P::decode(&mut reader, version)?;
            reader.finish(P::NAME)?;
            handler.call(ctx, packet)
        });

        self.entries.push(Entry {
            name: P::NAME,
            phase: P::PHASE,
            ids: P::IDS,
            dispatch,
        });
        self
    }

    /// Registers the tick handler, used for keep-alives and deadlines.
    #[must_use]
    pub fn on_tick<H: TickHandler<S>>(mut self, handler: H) -> Self {
        self.tick = Some(Arc::new(handler));
        self
    }

    /// Registers the handler that gets the last word when a connection ends badly.
    ///
    /// This is where a disconnect message comes from -- see [`Dispatcher::on_error`].
    #[must_use]
    pub fn on_error<H: ErrorHandler<S>>(mut self, handler: H) -> Self {
        self.on_error = Some(Arc::new(handler));
        self
    }

    /// Builds the dispatch tables.
    ///
    /// This is the only place a router can fail. Every ID is resolved and every collision found
    /// here, so nothing about dispatch can go wrong once a connection is running.
    ///
    /// It takes no version list. The versions that matter are the ones the registered packets
    /// themselves name, and asking the caller to enumerate the rest only invited them to leave one
    /// out -- with no signal until a client on that exact version could not log in.
    pub fn build(self) -> std::result::Result<Router<S>, BuildError> {
        if let Some(err) = self.invalid {
            return Err(err);
        }
        if self.entries.len() > MAX_PACKETS {
            return Err(BuildError::TooManyPackets {
                count: self.entries.len(),
                limit: MAX_PACKETS,
            });
        }

        let entries = self.entries.into_boxed_slice();
        let mut tables = Vec::with_capacity(breakpoints(&entries).len());
        for version in breakpoints(&entries) {
            tables.push((version, Arc::new(build_table(&entries, version)?)));
        }

        Ok(Router {
            unknown: self.unknown,
            entries,
            tables: tables.into_boxed_slice(),
            tick: self.tick,
            on_error: self.on_error,
        })
    }
}

/// The first out-of-order pair in an ID table, if there is one.
fn unordered(packet: &'static str, ids: &'static [(ProtocolVersion, i32)]) -> Option<BuildError> {
    ids.windows(2).find_map(|pair| {
        (pair[0].0 <= pair[1].0).then_some(BuildError::UnorderedIds {
            packet,
            previous: pair[0].0,
            version: pair[1].0,
        })
    })
}

/// The versions at which dispatch changes: every threshold any packet names, plus the floor.
///
/// Sorted ascending, so a lookup is a binary search and a version between two entries belongs to
/// the lower one.
fn breakpoints<S>(entries: &[Entry<S>]) -> Vec<ProtocolVersion> {
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

fn build_table<S>(
    entries: &[Entry<S>],
    version: ProtocolVersion,
) -> std::result::Result<Table, BuildError> {
    let mut by_phase: [Vec<Option<u16>>; Phase::COUNT] = Default::default();

    for (index, entry) in entries.iter().enumerate() {
        let Some(id) = entry.id(version) else {
            continue;
        };
        if !(0..=MAX_PACKET_ID).contains(&id) {
            return Err(BuildError::IdOutOfRange {
                packet: entry.name,
                id,
                version,
            });
        }

        let table = &mut by_phase[entry.phase.index()];
        let slot = id as usize;
        if table.len() <= slot {
            table.resize(slot + 1, None);
        }
        if let Some(existing) = table[slot] {
            return Err(BuildError::IdCollision {
                first: entries[existing as usize].name,
                second: entry.name,
                id,
                phase: entry.phase,
                version,
            });
        }
        // Bounded by `MAX_PACKETS`, checked before this runs.
        table[slot] = Some(index as u16);
    }

    Ok(Table {
        by_phase: by_phase.map(Vec::into_boxed_slice),
    })
}

/// A built set of packet handlers, independent of any single connection.
///
/// Wrap it in an [`Arc`] and share it across every connection.
pub struct Router<S> {
    unknown: UnknownPolicy,
    entries: Box<[Entry<S>]>,
    /// One table per version at which dispatch changes, ascending. Never empty: the floor is
    /// always present.
    tables: Box<[(ProtocolVersion, Arc<Table>)]>,
    tick: Option<Arc<dyn TickHandler<S>>>,
    on_error: Option<Arc<dyn ErrorHandler<S>>>,
}

// Derived `Debug` would demand `S: Debug` and would try to format the handlers, neither of which
// is meaningful. What is worth seeing is the shape of the table.
impl<S> std::fmt::Debug for Router<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("unknown", &self.unknown)
            .field("packets", &self.entries.len())
            .field("tables", &self.tables.len())
            .field("ticks", &self.tick.is_some())
            .field("handles_errors", &self.on_error.is_some())
            .finish()
    }
}

impl<S: 'static> Router<S> {
    /// Starts building a router.
    ///
    /// A router is not bound to a direction. [`Direction`](crate::packet::Direction) is part of a
    /// packet's identity and says which way it travels, but the table is keyed by phase and ID
    /// alone -- so a client, a server and a proxy all build one the same way. Registering both
    /// directions of one phase will collide on the IDs they share, which is the honest signal that
    /// a direction-keyed table is what that case needs.
    #[must_use]
    pub fn builder() -> RouterBuilder<S> {
        RouterBuilder {
            unknown: UnknownPolicy::default(),
            entries: Vec::new(),
            tick: None,
            on_error: None,
            invalid: None,
        }
    }

    /// Whether a tick handler is registered. A connection only arms its timer if there is one.
    #[must_use]
    pub fn ticks(&self) -> bool {
        self.tick.is_some()
    }

    /// The table covering `version`.
    ///
    /// A version that [`is_release`](ProtocolVersion::is_release) gets the table of the highest
    /// breakpoint at or below it. Anything else -- a snapshot, a negative number -- gets the floor,
    /// because it cannot be ordered against the thresholds in any way a codec could act on.
    fn table(&self, version: ProtocolVersion) -> Arc<Table> {
        let index = if version.is_release() {
            self.tables
                .partition_point(|(since, _)| version.at_least(*since))
                .saturating_sub(1)
        } else {
            0
        };
        // The floor is always present, so index 0 always exists.
        Arc::clone(&self.tables[index].1)
    }
}

/// A [`Router`] bound to one connection's protocol version: the [`Dispatcher`] a connection runs
/// on by default.
///
/// It holds two `Arc`s -- the router, which is shared by every connection, and the dispatch table
/// for the version this connection negotiated. Keeping the table here rather than looking it up per
/// frame is what makes dispatch two loads instead of a hash lookup and a pair of atomics: measured,
/// 1.8 ns against 14 ns. That is nothing at Passage's packet counts, but it is also free, and it is
/// the reason the connection needs to know nothing about tables.
pub struct RouterDispatcher<S> {
    router: Arc<Router<S>>,
    table: Arc<Table>,
}

// Derived `Clone` would demand `S: Clone`; both fields are `Arc`s and clone regardless.
impl<S> Clone for RouterDispatcher<S> {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
            table: Arc::clone(&self.table),
        }
    }
}

impl<S> std::fmt::Debug for RouterDispatcher<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterDispatcher")
            .field("router", &self.router)
            .finish_non_exhaustive()
    }
}

impl<S: 'static> RouterDispatcher<S> {
    /// Creates a dispatcher for `router`, before a version has been negotiated.
    ///
    /// Takes the router by value or as an [`Arc`] -- pass a clone of the same `Arc` for every
    /// connection, which is what [`serve`](crate::server::serve) does.
    ///
    /// It starts on the version-independent table, and
    /// [`Connection::new`](crate::conn::Connection::new) immediately rebinds it to the version its
    /// configuration starts on.
    #[must_use]
    pub fn new(router: impl Into<Arc<Router<S>>>) -> Self {
        let router = router.into();
        let table = router.table(ProtocolVersion::UNKNOWN);
        Self { router, table }
    }

    /// The router this dispatches to.
    #[must_use]
    pub fn router(&self) -> &Arc<Router<S>> {
        &self.router
    }
}

impl<S: 'static> Dispatcher<S> for RouterDispatcher<S> {
    fn set_version(&mut self, version: ProtocolVersion) {
        self.table = self.router.table(version);
    }

    fn dispatch(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let router = &*self.router;
        let phase = ctx.phase();

        let Some(index) = self.table.lookup(phase, id) else {
            if router.unknown == UnknownPolicy::Ignore {
                trace!(id, ?phase, "ignoring unhandled packet");
                return Ok(());
            }
            // The ID may well be a packet we know, just not one that belongs here. Saying so beats
            // reporting it as unknown, which sends whoever reads the log looking for the wrong bug.
            if let Some(other) = self.table.lookup_elsewhere(phase, id) {
                let entry = &router.entries[other as usize];
                return Err(ProtocolError::UnexpectedPacket {
                    packet: entry.name,
                    expected: entry.phase,
                    phase,
                }
                .into());
            }
            return Err(ProtocolError::UnknownPacket {
                phase,
                version: ctx.version(),
                id,
            }
            .into());
        };

        // Tracing lives here rather than on the connection, because this is where the name is
        // known -- a connection has an ID and a payload and nothing else.
        let entry = &router.entries[index as usize];
        trace!(packet = entry.name, ?phase, "dispatching packet");
        (entry.dispatch)(ctx, payload)
    }

    fn tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        match &self.router.tick {
            Some(handler) => handler.call(ctx),
            None => Ok(()),
        }
    }

    fn ticks(&self) -> bool {
        self.router.ticks()
    }

    fn on_error(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()> {
        match &self.router.on_error {
            Some(handler) => handler.call(ctx, ending),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::versions;

    struct Fake;

    fn entry<S>(ids: &'static [(ProtocolVersion, i32)]) -> Entry<S> {
        Entry {
            name: "Fake",
            phase: Phase::Login,
            ids,
            dispatch: Box::new(|_, _| Ok(())),
        }
    }

    #[test]
    fn breakpoints_are_the_versions_the_packets_name() {
        let entries: Vec<Entry<Fake>> = vec![
            entry(&[(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)]),
            entry(&[(versions::V1_20_5, 0x03)]),
            entry(&[(ProtocolVersion::UNKNOWN, 0x00)]),
        ];
        assert_eq!(
            breakpoints(&entries),
            vec![ProtocolVersion::UNKNOWN, versions::V1_20_5, versions::V26_2],
        );
    }

    #[test]
    fn an_unordered_id_table_is_a_build_error() {
        assert!(
            unordered(
                "Fake",
                &[(versions::V1_20_5, 0x02), (versions::V26_2, 0x05)]
            )
            .is_some()
        );
        assert!(
            unordered(
                "Fake",
                &[(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)]
            )
            .is_none()
        );
        // A version listed twice is also out of order: the second entry is unreachable.
        assert!(unordered("Fake", &[(versions::V26_2, 0x05), (versions::V26_2, 0x02)]).is_some());
    }
}
