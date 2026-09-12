//! Typed packet registration with erased dispatch.
//!
//! Registration is generic (`.on::<LoginStart>(on_login_start)`), so the handler receives a
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
//! client that happens to send the packet, and a connection allocates nothing: it holds an [`Arc`]
//! of the router and the index of the table for its version.
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
//! [`ProtocolVersion::placed`], which is also what the *outbound* ID lookup uses, so the two agree).
//!
//! # How a connection reaches this
//!
//! Not directly. A [`Connection`](crate::conn::Connection) depends on the
//! [`Dispatcher`] trait, and [`RouterDispatcher`] is the implementation that dispatches to a
//! router: one per connection, holding an [`Arc`] of the shared router plus the index of the table
//! for the version that connection negotiated. That is where the version-to-table binding lives,
//! and it is why the table type never leaves this module.

use crate::conn::{Ctx, Dispatcher, Ending};
use crate::error::{BuildError, ProtocolError, Result};
use crate::packet::{Packet, Phase};
use crate::server::MakeDispatcher;
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use std::sync::Arc;
use tracing::trace;

/// The highest packet ID a registration may claim.
///
/// This is a sanity ceiling, not the table's width: a table is sized by the IDs actually registered
/// in its phase, so raising this costs nothing for a server that only handles a handful of packets.
/// It has to sit above the Play phase, whose clientbound IDs already run past `0x80` -- Passage
/// never reaches that phase, but the driver is not Passage.
const MAX_PACKET_ID: i32 = 1023;

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
type ErasedHandler<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<()> + Send + Sync>;

/// The handler that runs on every tick instead of on a packet. Shared, so a dispatcher can hold the
/// router rather than a copy of it.
type TickHandler<S> = Arc<dyn for<'c> Fn(Ctx<'c, S>) -> Result<()> + Send + Sync>;

/// The handler that runs when a connection ends for a reason nobody asked for.
///
/// See [`Dispatcher::on_error`] for what it may do and what it is called for.
type ErrorHandler<S> = Arc<dyn for<'c> Fn(Ctx<'c, S>, &Ending) -> Result<()> + Send + Sync>;

struct Entry<S> {
    name: &'static str,
    phase: Phase,
    /// The packet's own ID table, newest first. Read both to resolve an ID and to find the
    /// versions at which dispatch changes.
    ids: &'static [(ProtocolVersion, i32)],
    dispatch: ErasedHandler<S>,
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
    tick: Option<TickHandler<S>>,
    on_error: Option<ErrorHandler<S>>,
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
    ///
    /// The packet is the only thing worth naming -- `.on::<LoginStart>(on_login_start)` -- and
    /// pairing it with a handler that takes something else does not compile.
    #[must_use]
    pub fn on<P: Packet>(
        mut self,
        handler: impl Fn(Ctx<'_, S>, P) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        // Recorded rather than returned on, so the entry set stays complete: a collision between
        // two *other* packets is still found, and `build` reports whichever mistake came first.
        if self.entries.len() >= MAX_PACKETS {
            self.invalid.get_or_insert(BuildError::TooManyPackets {
                count: self.entries.len() + 1,
                limit: MAX_PACKETS,
            });
            return self;
        }
        if let Some(err) = unordered(P::NAME, P::IDS) {
            self.invalid.get_or_insert(err);
        }

        // The trailing-bytes check lives here, so it runs for every packet and no hand-written
        // decoder has to remember it.
        let dispatch: ErasedHandler<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let version = ctx.version();
            let mut reader = Reader::new(payload, ctx.limits());
            let packet = P::decode(&mut reader, version)?;
            reader.finish(P::NAME)?;
            handler(ctx, packet)
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
    pub fn on_tick(
        mut self,
        handler: impl Fn(Ctx<'_, S>) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.tick = Some(Arc::new(handler));
        self
    }

    /// Registers the handler that gets the last word when a connection ends badly.
    ///
    /// This is where a disconnect message comes from -- see [`Dispatcher::on_error`].
    #[must_use]
    pub fn on_error(
        mut self,
        handler: impl Fn(Ctx<'_, S>, &Ending) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
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

        let entries = self.entries.into_boxed_slice();
        let breakpoints = breakpoints(&entries);
        let mut tables = Vec::with_capacity(breakpoints.len());
        for version in breakpoints {
            tables.push((version, build_table(&entries, version)?));
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
        let Some(id) = crate::packet::ids(version, entry.ids) else {
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
    tables: Box<[(ProtocolVersion, Table)]>,
    tick: Option<TickHandler<S>>,
    on_error: Option<ErrorHandler<S>>,
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

    /// Where the table covering `version` sits in [`tables`](Router::tables).
    ///
    /// The highest breakpoint at or below it. A version that cannot be ordered against the
    /// thresholds at all -- a snapshot, a negative number -- is [`placed`](ProtocolVersion::placed)
    /// on the floor first, which is the same rule [`ids`](crate::packet::ids) applies on the way
    /// out, so what a connection accepts and what it sends stay the same set.
    ///
    /// An index rather than the table: a [`RouterDispatcher`] already holds the router the table
    /// lives in, so nothing it could point at can outlive it and there is no lifetime to prove with
    /// a reference count.
    fn table(&self, version: ProtocolVersion) -> usize {
        let version = version.placed();
        self.tables
            .partition_point(|(since, _)| version.at_least(*since))
            // The floor is always present, so index 0 always exists.
            .saturating_sub(1)
    }
}

/// A [`Router`] bound to one connection's protocol version: the [`Dispatcher`] a connection runs
/// on by default.
///
/// One `Arc` -- the router, shared by every connection -- plus the index of the table for the
/// version this connection negotiated. Resolving that index once per version change rather than
/// once per frame is the point: it turns dispatch into a pair of indexed loads, where looking the
/// version up per frame would repeat a binary search over the breakpoints for every packet. At
/// Passage's packet counts the difference does not matter; what it buys is that the connection
/// needs to know nothing about tables.
pub struct RouterDispatcher<S> {
    router: Arc<Router<S>>,
    /// Index into `router.tables`, rebound by [`set_version`](Dispatcher::set_version).
    table: usize,
}

// Derived `Clone` would demand `S: Clone`; an `Arc` and an index clone regardless.
impl<S> Clone for RouterDispatcher<S> {
    fn clone(&self) -> Self {
        Self {
            router: Arc::clone(&self.router),
            table: self.table,
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
    /// [`Connection::builder`](crate::conn::Connection::builder) immediately rebinds it to the version its
    /// configuration starts on.
    #[must_use]
    pub fn new(router: impl Into<Arc<Router<S>>>) -> Self {
        let router = router.into();
        let table = router.table(ProtocolVersion::UNKNOWN);
        Self { router, table }
    }
}

/// The ordinary case: every connection gets a [`RouterDispatcher`] over the same shared router.
///
/// Implemented here rather than in [`server`](crate::server), which declares the trait, because
/// that is the direction every other seam in the crate runs: the consumer declares, the provider
/// implements for its own type. It is also why nothing in `server` names a [`Router`].
impl<S: 'static> MakeDispatcher<S> for Arc<Router<S>> {
    type Dispatcher = RouterDispatcher<S>;

    fn make(&self) -> RouterDispatcher<S> {
        RouterDispatcher::new(Arc::clone(self))
    }
}

impl<S: 'static> Dispatcher<S> for RouterDispatcher<S> {
    fn set_version(&mut self, version: ProtocolVersion) {
        self.table = self.router.table(version);
    }

    fn dispatch(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let router = &*self.router;
        let table = &router.tables[self.table].1;
        let phase = ctx.phase();

        let Some(index) = table.lookup(phase, id) else {
            if router.unknown == UnknownPolicy::Ignore {
                trace!(id, ?phase, "ignoring unhandled packet");
                return Ok(());
            }
            // The ID may well be a packet we know, just not one that belongs here. Saying so beats
            // reporting it as unknown, which sends whoever reads the log looking for the wrong bug.
            if let Some(other) = table.lookup_elsewhere(phase, id) {
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
            Some(handler) => handler(ctx),
            None => Ok(()),
        }
    }

    fn ticks(&self) -> bool {
        self.router.ticks()
    }

    fn on_error(&self, ctx: Ctx<'_, S>, ending: &Ending) -> Result<()> {
        match &self.router.on_error {
            Some(handler) => handler(ctx, ending),
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
