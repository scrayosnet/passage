//! Typed packet registration with erased dispatch.
//!
//! Registration is generic (`.on::<LoginStart, _>(on_login_start)`), so the handler receives a
//! decoded packet and a wrong pairing does not compile. Storage is erased, so dispatch is a table
//! lookup and one virtual call -- and, crucially, adding a packet does not change any trait, which
//! means it does not break everything that already exists.
//!
//! This is the axum/tonic shape rather than the "one big trait with a method per packet" shape. The
//! trade-off is discussed in `docs/03-dispatch.md`.
//!
//! # Tables are built once
//!
//! [`RouterBuilder::build`] resolves every registered packet's ID at every supported version and
//! produces the dispatch tables up front. So an ID collision is a startup failure rather than a
//! runtime error on the first client that happens to send the packet, and a connection allocates
//! nothing: it borrows an [`Arc`] of the table for its version.
//!
//! A version with no table of its own -- anything outside the supported range, including the
//! garbage a scanner sends -- gets the version-independent table, which is exactly the packets
//! whose ID table starts at [`ProtocolVersion::UNKNOWN`]. That is enough to answer a status ping
//! and refuse a login, and it means the `i32` version space cannot cost memory.

use crate::conn::Ctx;
use crate::error::{BuildError, ProtocolError, Result};
use crate::packet::{Direction, Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use std::collections::HashMap;
use std::sync::Arc;

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

/// A decoder and handler pair with the packet type erased.
type Dispatcher<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<()> + Send + Sync>;

struct Entry<S> {
    name: &'static str,
    phase: Phase,
    id: fn(ProtocolVersion) -> Option<i32>,
    dispatch: Dispatcher<S>,
}

/// The dispatch table for one protocol version: `phase -> id -> index into the router's entries`.
///
/// Indices rather than pointers, so a lookup is two loads with no reference count to touch, and so
/// the table itself is free of `S` and can be shared as-is.
pub(crate) struct Table {
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
    inbound: Direction,
    unknown: UnknownPolicy,
    entries: Vec<Entry<S>>,
    tick: Option<Arc<dyn TickHandler<S>>>,
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
        if P::DIRECTION != self.inbound {
            self.invalid.get_or_insert(BuildError::WrongDirection {
                packet: P::NAME,
                actual: P::DIRECTION,
                expected: self.inbound,
            });
            return self;
        }

        // The trailing-bytes check lives here, so it runs for every packet and no hand-written
        // decoder has to remember it.
        let dispatch: Dispatcher<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let version = ctx.version();
            let mut reader = Reader::new(payload, ctx.limits());
            let packet = P::decode(&mut reader, version)?;
            reader.finish(P::NAME)?;
            handler.call(ctx, packet)
        });

        self.entries.push(Entry {
            name: P::NAME,
            phase: P::PHASE,
            id: P::id,
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

    /// Builds the dispatch tables for `versions`.
    ///
    /// This is the only place a router can fail. Every ID is resolved and every collision found
    /// here, so nothing about dispatch can go wrong once a connection is running.
    pub fn build(
        self,
        versions: impl IntoIterator<Item = ProtocolVersion>,
    ) -> std::result::Result<Router<S>, BuildError> {
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
        let fallback = Arc::new(build_table(&entries, ProtocolVersion::UNKNOWN)?);

        let mut tables = HashMap::new();
        for version in versions {
            if version == ProtocolVersion::UNKNOWN {
                continue;
            }
            tables.insert(version, Arc::new(build_table(&entries, version)?));
        }

        Ok(Router {
            inbound: self.inbound,
            unknown: self.unknown,
            entries,
            tables,
            fallback,
            tick: self.tick,
        })
    }
}

fn build_table<S>(
    entries: &[Entry<S>],
    version: ProtocolVersion,
) -> std::result::Result<Table, BuildError> {
    let mut by_phase: [Vec<Option<u16>>; Phase::COUNT] = Default::default();

    for (index, entry) in entries.iter().enumerate() {
        let Some(id) = (entry.id)(version) else {
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
    inbound: Direction,
    unknown: UnknownPolicy,
    entries: Box<[Entry<S>]>,
    tables: HashMap<ProtocolVersion, Arc<Table>>,
    fallback: Arc<Table>,
    tick: Option<Arc<dyn TickHandler<S>>>,
}

// Derived `Debug` would demand `S: Debug` and would try to format the handlers, neither of which
// is meaningful. What is worth seeing is the shape of the table.
impl<S> std::fmt::Debug for Router<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router")
            .field("inbound", &self.inbound)
            .field("unknown", &self.unknown)
            .field("packets", &self.entries.len())
            .field("versions", &self.tables.len())
            .field("ticks", &self.tick.is_some())
            .finish()
    }
}

/// What dispatching one frame did.
pub(crate) enum Dispatched {
    /// A handler ran.
    Handled(&'static str),

    /// No handler was registered and the policy is to ignore it.
    Ignored,
}

impl<S: 'static> Router<S> {
    /// Starts building a router for a peer that receives packets travelling in `inbound` direction
    /// -- [`Direction::Serverbound`] for a server, [`Direction::Clientbound`] for a client.
    #[must_use]
    pub fn builder(inbound: Direction) -> RouterBuilder<S> {
        RouterBuilder {
            inbound,
            unknown: UnknownPolicy::default(),
            entries: Vec::new(),
            tick: None,
            invalid: None,
        }
    }

    /// The direction this router receives packets in.
    #[must_use]
    pub fn inbound(&self) -> Direction {
        self.inbound
    }

    /// The configured unknown-packet policy.
    #[must_use]
    pub fn unknown_policy(&self) -> UnknownPolicy {
        self.unknown
    }

    /// Whether a tick handler is registered. A connection only arms its timer if there is one.
    #[must_use]
    pub fn ticks(&self) -> bool {
        self.tick.is_some()
    }

    pub(crate) fn table(&self, version: ProtocolVersion) -> Arc<Table> {
        Arc::clone(self.tables.get(&version).unwrap_or(&self.fallback))
    }

    pub(crate) fn tick_handler(&self) -> Option<&Arc<dyn TickHandler<S>>> {
        self.tick.as_ref()
    }

    /// Decodes and dispatches one frame.
    pub(crate) fn dispatch(
        &self,
        table: &Table,
        ctx: Ctx<'_, S>,
        id: i32,
        payload: &[u8],
    ) -> Result<Dispatched> {
        let phase = ctx.phase();
        let Some(index) = table.lookup(phase, id) else {
            if self.unknown == UnknownPolicy::Ignore {
                return Ok(Dispatched::Ignored);
            }
            // The ID may well be a packet we know, just not one that belongs here. Saying so beats
            // reporting it as unknown, which sends whoever reads the log looking for the wrong bug.
            if let Some(other) = table.lookup_elsewhere(phase, id) {
                let entry = &self.entries[other as usize];
                return Err(ProtocolError::UnexpectedPacket {
                    packet: entry.name,
                    expected: entry.phase,
                    phase,
                }
                .into());
            }
            return Err(ProtocolError::UnknownPacket {
                phase,
                direction: self.inbound,
                version: ctx.version(),
                id,
            }
            .into());
        };

        let entry = &self.entries[index as usize];
        (entry.dispatch)(ctx, payload)?;
        Ok(Dispatched::Handled(entry.name))
    }
}
