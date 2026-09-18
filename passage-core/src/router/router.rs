use crate::connection::{ConnectionError, Ctx, DispatchError};
use crate::packet::{Packet, check_ids_unordered};
use crate::router::{Entry, ErasedHandler, ErrorHandler, RouterError, Table, TickHandler};
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use anyhow::Context;
use std::sync::Arc;

/// The most packets a router can hold, bounded by the table's index width.
const MAX_PACKETS: usize = u16::MAX as usize;

/// The [`UnknownPolicy`] configures how unknown packets are handled. By default, unknown packets
/// raise an error. This is to prevent malicious and incompatible clients from accessing the driver.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum UnknownPolicy {
    /// Reject unknown packets and raise an error.
    #[default]
    Reject,

    /// Ignore unknown packets. Useful for minimal drivers.
    Ignore,
}

/// [`RouterBuilder`] builds a [`Router`] that can be shared across connections.
pub struct RouterBuilder<S> {
    /// The policy for handling unknown packets.
    unknown: UnknownPolicy,

    /// The list of packet handlers.
    entries: Vec<Entry<S>>,

    /// The tick handler.
    tick: Option<TickHandler<S>>,

    /// The error handler.
    on_error: Option<ErrorHandler<S>>,
}

impl<S: 'static> RouterBuilder<S> {
    /// Sets what happens to packets without a handler.
    #[must_use]
    pub fn unknown(mut self, policy: UnknownPolicy) -> Self {
        self.unknown = policy;
        self
    }

    /// Registers a new packet handler.
    ///
    /// # Errors
    ///
    /// Returns an error if the router is already saturated or the packet IDs are misconfigured.
    pub fn on<P: Packet>(
        mut self,
        handler: impl Fn(Ctx<'_, S>, P) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Result<Self, RouterError> {
        // Ensure that the packet may be registered to the router. The router can store at most
        // `MAX_PACKETS` packets. The packet IDs must also be ordered ascending by their protocol version.
        if self.entries.len() >= MAX_PACKETS {
            return Err(RouterError::TooManyPackets {
                count: self.entries.len() + 1,
                limit: MAX_PACKETS,
            });
        }

        if let Some((previous, version)) = check_ids_unordered(P::IDS) {
            return Err(RouterError::UnorderedIds {
                packet: P::NAME,
                previous,
                version,
            });
        }

        // Creates the erased handler from the given handler. It tries to decode the packet from the
        // buffer and passes it to the handler. Packets are generally defined together with the
        // handlers. As such, decoding handlers are wrapped as dispatch errors (i.e., anyhow).
        let dispatch: ErasedHandler<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let mut reader = Reader::new(payload).with_options(ctx.handle.options());
            reader
                .var_int("packet_id")
                .with_context(|| format!("packet {} id failed to decode", P::NAME))?;
            let packet = P::decode(&mut reader, ctx.version)
                .with_context(|| format!("packet {} failed to decode", P::NAME))?;
            reader
                .finish(P::NAME)
                .with_context(|| format!("packet {} failed to consume buffer", P::NAME))?;
            handler(ctx, packet)
        });
        self.entries.push(Entry {
            name: P::NAME,
            phase: P::PHASE,
            ids: P::IDS,
            dispatch,
        });
        Ok(self)
    }

    /// Registers the tick handler, used for keep alive packets and deadlines.
    #[must_use]
    pub fn on_tick(
        mut self,
        handler: impl Fn(Ctx<'_, S>) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self {
        self.tick = Some(Arc::new(handler));
        self
    }

    /// Registers the error handler, used to track errors and send (unexpected) disconnect packets.
    #[must_use]
    pub fn on_error(
        mut self,
        handler: impl Fn(Ctx<'_, S>, &ConnectionError) -> Result<(), DispatchError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.on_error = Some(Arc::new(handler));
        self
    }

    /// Builds the (immutable) router from the handlers, optimizing the handler layout for fast dispatch.
    /// A router should have at least one handler registered.
    pub fn build(self) -> Router<S> {
        let entries = self.entries.into_boxed_slice();
        let breakpoints = Table::breakpoints(&entries);
        let mut tables = Vec::with_capacity(breakpoints.len());
        for version in breakpoints {
            tables.push((version, Table::new(entries.as_ref(), version)));
        }
        Router {
            unknown: self.unknown,
            entries,
            tables: tables.into_boxed_slice(),
            tick: self.tick,
            on_error: self.on_error,
        }
    }
}

/// A [`Router`] holds a list of (packet) handlers for each registered protocol version. It can be
/// used to make connection dispatcher.
pub struct Router<S> {
    /// The policy for handling unknown packets.
    pub(crate) unknown: UnknownPolicy,

    /// The registered packet handlers.
    pub(crate) entries: Box<[Entry<S>]>,

    /// The routing tables per protocol version (ordered ascending). Each table holds a list of indices
    /// into the [`entries`] array (per phase and packet ID). The layout is used to perform binary
    /// searches. The first table should hold the initial routing table (i.e., [`ProtocolVersion::UNKNOWN`]).
    pub(crate) tables: Box<[(ProtocolVersion, Table)]>,

    /// The tick handler.
    pub(crate) tick: Option<TickHandler<S>>,

    /// The error handler.
    pub(crate) on_error: Option<ErrorHandler<S>>,
}

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
    /// Creates a new [`RouterBuilder`]. It is used to create a new [`Router`].
    #[must_use]
    pub fn builder() -> RouterBuilder<S> {
        RouterBuilder {
            unknown: UnknownPolicy::default(),
            entries: Vec::new(),
            tick: None,
            on_error: None,
        }
    }

    /// Computes the internal index for the table that is used for this protocol version. The table
    /// list does not list every protocol version, but breakpoints based on the registered packet
    /// handlers. This uses the internal table layout to perform an efficient binary search.
    pub(crate) fn table(&self, version: ProtocolVersion) -> usize {
        let version = version.placed();
        self.tables
            .partition_point(|(since, _)| version.at_least(*since))
            // The floor is always present, so index 0 always exists.
            .saturating_sub(1)
    }
}
