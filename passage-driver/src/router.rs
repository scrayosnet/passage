//! Typed packet registration with erased dispatch.
//!
//! Registration is generic (`router.on::<LoginStart, _>(handle_login_start)`), so the handler
//! receives a decoded packet and a wrong pairing does not compile. Storage is erased, so dispatch
//! is a table lookup and one virtual call -- and, crucially, adding a packet does not change any
//! trait, which means it does not break every implementor.
//!
//! This is the axum/tonic shape rather than the "one big trait with a method per packet" shape. The
//! trade-off is discussed in `docs/03-dispatch.md`.

use crate::conn::Ctx;
use crate::error::{Error, InternalError, ProtocolError, Result};
use crate::flow::{Flow, Outcome};
use crate::packet::{Direction, Packet, Phase};
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use std::sync::Arc;

/// The highest packet ID the dispatch table covers.
///
/// Real IDs are far below this; anything above is answered by the unknown-packet policy without
/// allocating a table for it.
const MAX_PACKET_ID: i32 = 255;

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
/// Blanket-implemented for every `fn(Ctx<'_, S>, P) -> Outcome<S>`, including closures.
pub trait Handler<S, P>: Send + Sync + 'static {
    /// Handles one decoded packet.
    fn call(&self, ctx: Ctx<'_, S>, packet: P) -> Outcome<S>;
}

impl<S, P, F> Handler<S, P> for F
where
    F: Fn(Ctx<'_, S>, P) -> Outcome<S> + Send + Sync + 'static,
{
    fn call(&self, ctx: Ctx<'_, S>, packet: P) -> Outcome<S> {
        self(ctx, packet)
    }
}

/// A handler that runs on every tick instead of on a packet.
pub trait TickHandler<S>: Send + Sync + 'static {
    /// Handles one tick.
    fn call(&self, ctx: Ctx<'_, S>) -> Outcome<S>;
}

impl<S, F> TickHandler<S> for F
where
    F: Fn(Ctx<'_, S>) -> Outcome<S> + Send + Sync + 'static,
{
    fn call(&self, ctx: Ctx<'_, S>) -> Outcome<S> {
        self(ctx)
    }
}

type Erased<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Outcome<S> + Send + Sync>;

struct Entry<S> {
    name: &'static str,
    phase: Phase,
    id: fn(ProtocolVersion) -> Option<i32>,
    handler: Erased<S>,
}

/// A set of packet handlers, independent of any single connection or version.
///
/// Build it once at startup, wrap it in an [`Arc`] and share it across every connection.
pub struct Router<S> {
    inbound: Direction,
    unknown: UnknownPolicy,
    entries: Vec<Arc<Entry<S>>>,
    tick: Option<Arc<dyn TickHandler<S>>>,
}

impl<S: 'static> Router<S> {
    /// Creates a router for a peer that receives packets travelling in `inbound` direction --
    /// [`Direction::Serverbound`] for a server, [`Direction::Clientbound`] for a client.
    #[must_use]
    pub fn new(inbound: Direction) -> Self {
        Self {
            inbound,
            unknown: UnknownPolicy::default(),
            entries: Vec::new(),
            tick: None,
        }
    }

    /// Sets what happens to packets without a handler.
    #[must_use]
    pub fn unknown(mut self, policy: UnknownPolicy) -> Self {
        self.unknown = policy;
        self
    }

    /// The configured unknown-packet policy.
    #[must_use]
    pub fn unknown_policy(&self) -> UnknownPolicy {
        self.unknown
    }

    /// The direction this router receives packets in.
    #[must_use]
    pub fn inbound(&self) -> Direction {
        self.inbound
    }

    /// Registers a handler for one packet type.
    ///
    /// # Panics
    ///
    /// If `P` travels in the wrong direction for this router. That is a wiring mistake that cannot
    /// depend on input, so failing loudly at startup is better than a table that silently never
    /// matches.
    #[must_use]
    pub fn on<P, H>(mut self, handler: H) -> Self
    where
        P: Packet,
        H: Handler<S, P>,
    {
        assert_eq!(
            P::DIRECTION,
            self.inbound,
            "packet `{}` travels {:?} but this router receives {:?} packets",
            P::NAME,
            P::DIRECTION,
            self.inbound,
        );

        let handler = Arc::new(handler);
        let erased: Erased<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let version = ctx.version();
            let mut reader = Reader::new(payload, ctx.limits());
            match P::decode(&mut reader, version) {
                Ok(packet) => handler.call(ctx, packet),
                Err(err) => Flow::Ready(Err(err)),
            }
        });

        self.entries.push(Arc::new(Entry {
            name: P::NAME,
            phase: P::PHASE,
            id: P::id,
            handler: erased,
        }));
        self
    }

    /// Registers the tick handler, used for keep-alives and deadlines.
    #[must_use]
    pub fn on_tick<H: TickHandler<S>>(mut self, handler: H) -> Self {
        self.tick = Some(Arc::new(handler));
        self
    }

    pub(crate) fn tick_handler(&self) -> Option<&Arc<dyn TickHandler<S>>> {
        self.tick.as_ref()
    }

    /// Builds the dispatch table for one protocol version.
    ///
    /// Done once per connection (and again if the version changes, which happens exactly once, at
    /// the handshake). Lookup afterwards is two indexing operations.
    pub fn bind(&self, version: ProtocolVersion) -> Result<Bound<S>> {
        let mut tables: Vec<Vec<Option<Arc<Entry<S>>>>> =
            (0..Phase::COUNT).map(|_| Vec::new()).collect();

        for entry in &self.entries {
            let Some(id) = (entry.id)(version) else {
                continue;
            };
            if !(0..=MAX_PACKET_ID).contains(&id) {
                return Err(InternalError::Handler(
                    format!(
                        "packet `{}` has out-of-range id {id} in version {version}",
                        entry.name
                    )
                    .into(),
                )
                .into());
            }

            let table = &mut tables[entry.phase.index()];
            let index = id as usize;
            if table.len() <= index {
                table.resize(index + 1, None);
            }
            if let Some(existing) = &table[index] {
                return Err(InternalError::Handler(
                    format!(
                        "packets `{}` and `{}` both claim id {id:#04x} in phase {:?} of version \
                         {version}",
                        existing.name, entry.name, entry.phase,
                    )
                    .into(),
                )
                .into());
            }
            table[index] = Some(Arc::clone(entry));
        }

        Ok(Bound { version, tables })
    }

    /// Checks that the router binds cleanly for every given version.
    ///
    /// Call this at startup over the supported version range: an ID collision introduced by a new
    /// packet is then a boot failure instead of a runtime error on the first client that hits it.
    pub fn validate(&self, versions: impl IntoIterator<Item = ProtocolVersion>) -> Result<()> {
        for version in versions {
            self.bind(version)?;
        }
        Ok(())
    }
}

/// A dispatch table for one protocol version.
pub struct Bound<S> {
    version: ProtocolVersion,
    tables: Vec<Vec<Option<Arc<Entry<S>>>>>,
}

impl<S> Bound<S> {
    /// The version this table was built for.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    fn lookup(&self, phase: Phase, id: i32) -> Option<Arc<Entry<S>>> {
        let table = self.tables.get(phase.index())?;
        let index = usize::try_from(id).ok()?;
        table.get(index)?.clone()
    }
}

/// The result of dispatching one frame.
pub(crate) enum Dispatch<S> {
    /// A handler ran; this is its flow.
    Handled {
        name: &'static str,
        flow: Outcome<S>,
    },

    /// No handler was registered and the policy is to ignore it.
    Ignored,
}

impl<S: 'static> Bound<S> {
    /// Decodes and dispatches one frame.
    pub(crate) fn dispatch(
        &self,
        ctx: Ctx<'_, S>,
        phase: Phase,
        direction: Direction,
        id: i32,
        payload: &[u8],
        unknown: UnknownPolicy,
    ) -> Result<Dispatch<S>> {
        let Some(entry) = self.lookup(phase, id) else {
            return match unknown {
                UnknownPolicy::Ignore => Ok(Dispatch::Ignored),
                UnknownPolicy::Reject => Err(Error::Protocol(ProtocolError::UnknownPacket {
                    phase,
                    direction,
                    version: self.version,
                    id,
                })),
            };
        };

        let flow = (entry.handler)(ctx, payload);
        Ok(Dispatch::Handled {
            name: entry.name,
            flow,
        })
    }
}
