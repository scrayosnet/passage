use crate::connection::{ConnectionError, Ctx, DispatchError};
use crate::packet::{Packet, check_ids_unordered};
use crate::router::{
    Entry, ErasedHandler, ErrorHandler, OpenHandler, RouterError, Table, TickHandler,
};
use crate::version::ProtocolVersion;
use crate::wire::Reader;
use anyhow::anyhow;
use std::sync::Arc;

/// The most packets a router can hold, bounded by the table's index width.
const MAX_PACKETS: usize = u16::MAX as usize;

/// The label a payload that does not decode is reported under.
const MALFORMED: &str = "malformed_packet";

/// Raises a failure for a payload the peer sent that the packet cannot account for.
fn malformed(packet: &'static str, what: &str, cause: &dyn std::fmt::Display) -> DispatchError {
    DispatchError::peer(MALFORMED, anyhow!("packet `{packet}` {what}: {cause}"))
}

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

    /// The open handler.
    on_open: Option<OpenHandler<S>>,

    /// The tick handler.
    tick: Option<TickHandler<S>>,

    /// The error handler.
    on_error: Option<ErrorHandler<S>>,
}

impl<S> std::fmt::Debug for RouterBuilder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterBuilder")
            .field("unknown", &self.unknown)
            .field("packets", &self.entries.len())
            .field("handles_open", &self.on_open.is_some())
            .field("ticks", &self.tick.is_some())
            .field("handles_errors", &self.on_error.is_some())
            .finish()
    }
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
        // handlers. As such, handler errors are wrapped as dispatch errors (i.e., anyhow).
        let dispatch: ErasedHandler<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
            let mut reader = Reader::new(payload).with_options(ctx.handle.options());
            reader
                .var_int("packet_id")
                .map_err(|err| malformed(P::NAME, "has no ID", &err))?;
            let packet = P::decode(&mut reader, ctx.version)
                .map_err(|err| malformed(P::NAME, "failed to decode", &err))?;
            reader
                .finish(P::NAME)
                .map_err(|err| malformed(P::NAME, "was not fully consumed", &err))?;
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

    /// Registers the open handler, used by client implementations to send the initial packet.
    #[must_use]
    pub fn on_open(
        mut self,
        handler: impl Fn(Ctx<'_, S>) -> Result<(), DispatchError> + Send + Sync + 'static,
    ) -> Self {
        self.on_open = Some(Arc::new(handler));
        self
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
            on_open: self.on_open,
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

    /// The open handler.
    pub(crate) on_open: Option<OpenHandler<S>>,

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
            .field("handles_open", &self.on_open.is_some())
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
            on_open: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use crate::version::versions;
    use crate::wire::{Reader, WireResult, Writer};

    /// A packet anchored at the floor: the kind that answers a status ping from any client.
    struct Anchored;

    impl Packet for Anchored {
        const NAME: &'static str = "Anchored";
        const PHASE: Phase = Phase::Status;
        const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

        fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            Ok(())
        }
    }

    /// A packet whose ID moved, so the router has a reason to hold more than one table.
    struct Moved;

    impl Packet for Moved {
        const NAME: &'static str = "Moved";
        const PHASE: Phase = Phase::Login;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)];

        fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            Ok(())
        }
    }

    /// An ID table written oldest-first, which resolves every newer version to the wrong ID.
    struct Backwards;

    impl Packet for Backwards {
        const NAME: &'static str = "Backwards";
        const PHASE: Phase = Phase::Login;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V1_20_5, 0x40), (versions::V26_2, 0x41)];

        fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            Ok(())
        }
    }

    fn router() -> Router<()> {
        Router::<()>::builder()
            .on::<Anchored>(|_ctx, _packet| Ok(()))
            .expect("registers")
            .on::<Moved>(|_ctx, _packet| Ok(()))
            .expect("registers")
            .build()
    }

    #[test]
    fn a_router_holds_one_table_per_change_and_nothing_between() {
        let router = router();
        // The floor, 1.20.5 and 26.2 -- the versions the packets themselves name.
        assert_eq!(router.tables.len(), 3);

        // Everything between two thresholds dispatches against the same table.
        assert_eq!(router.table(ProtocolVersion::UNKNOWN), 0);
        assert_eq!(router.table(ProtocolVersion::new(765)), 0);
        assert_eq!(router.table(versions::V1_20_5), 1);
        assert_eq!(
            router.table(ProtocolVersion::new(767)),
            1,
            "1.21 is 1.20.5's table"
        );
        assert_eq!(router.table(versions::V26_2), 2);
        assert_eq!(router.table(ProtocolVersion::new(999)), 2);
    }

    #[test]
    fn a_version_nothing_can_place_dispatches_against_the_floor() {
        // The same `placed` the encoder uses, because an ID we would accept has to be one we would
        // send: a snapshot compares above every release and must not be handed the newest table.
        let router = router();
        for version in [
            ProtocolVersion::new(0x4000_0000 | 132),
            ProtocolVersion::new(-1),
            ProtocolVersion::new(i32::MIN),
        ] {
            assert_eq!(router.table(version), 0, "{version}");
        }
    }

    #[test]
    fn a_packet_is_looked_up_by_the_id_its_version_gives_it() {
        let router = router();
        let table = |version| &router.tables[router.table(version)].1;

        assert_eq!(table(versions::V1_20_5).lookup(Phase::Login, 0x02), Some(1));
        assert_eq!(table(versions::V1_20_5).lookup(Phase::Login, 0x05), None);
        assert_eq!(table(versions::V26_2).lookup(Phase::Login, 0x05), Some(1));
        assert_eq!(table(versions::V26_2).lookup(Phase::Login, 0x02), None);
        // The anchored packet is in every table, which is what lets any client be answered.
        for version in [ProtocolVersion::UNKNOWN, versions::V1_20_5, versions::V26_2] {
            assert_eq!(table(version).lookup(Phase::Status, 0x00), Some(0));
        }
    }

    #[test]
    fn an_id_table_written_the_wrong_way_round_is_refused_at_registration() {
        // It must surface at startup, not on the first client that happens to send one of them.
        let err = Router::<()>::builder()
            .on::<Backwards>(|_ctx, _packet| Ok(()))
            .expect_err("must reject");
        assert!(
            matches!(
                err,
                RouterError::UnorderedIds {
                    packet: "Backwards",
                    previous,
                    version,
                } if previous == versions::V1_20_5 && version == versions::V26_2
            ),
            "{err}",
        );
    }

    #[test]
    fn the_hooks_a_router_has_are_visible_on_it() {
        let bare = Router::<()>::builder().build();
        assert!(bare.on_open.is_none());
        assert!(bare.tick.is_none());
        assert!(bare.on_error.is_none());
        assert_eq!(bare.unknown, UnknownPolicy::Reject, "the safe default");
        assert_eq!(
            format!("{bare:?}"),
            "Router { unknown: Reject, packets: 0, tables: 1, handles_open: false, ticks: false, \
             handles_errors: false }",
        );

        let full = Router::<()>::builder()
            .unknown(UnknownPolicy::Ignore)
            .on_open(|_ctx| Ok(()))
            .on_tick(|_ctx| Ok(()))
            .on_error(|_ctx, _error| Ok(()))
            .build();
        assert!(full.on_open.is_some());
        assert!(full.tick.is_some());
        assert!(full.on_error.is_some());
        assert_eq!(full.unknown, UnknownPolicy::Ignore);
    }
}
