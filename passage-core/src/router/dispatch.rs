use crate::connection::{ConnectionError, Ctx, DispatchError, Dispatcher, MakeDispatcher};
use crate::router::{Router, UnknownPolicy};
use crate::version::ProtocolVersion;
use anyhow::anyhow;
use std::sync::Arc;
use tracing::trace;

/// A stateful [`Dispatcher`] based on a [`Router`]. It uses the router's tables to dispatch packets.
pub struct RouterDispatcher<S> {
    /// The router that this dispatcher dispatches against.
    router: Arc<Router<S>>,

    /// The cached index into `router.tables` for the last submitted protocol version. By caching the
    /// table index, we only have to use a single (binary) search to get the routing table per protocol
    /// version change (generally only once per connection). By default, it points to the fallback
    /// table which contains only the initial handshake packet.
    table: (ProtocolVersion, usize),
}

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
    /// Creates a dispatcher for `router`. It initially points to the first table in the router's
    /// table array (i.e., the [`ProtocolVersion::UNKNOWN`] if available).
    #[must_use]
    pub fn new(router: impl Into<Arc<Router<S>>>) -> Self {
        // We can assume that the `ProtocolVersion::UNKNOWN` table is either the first table or not
        // configured. In both cases, this will work: Either it is correct or the caller will re-compute
        // it when calling with a different protocol version.
        Self {
            router: router.into(),
            table: (ProtocolVersion::UNKNOWN, 0),
        }
    }
}

impl<S: 'static> MakeDispatcher<S> for Arc<Router<S>> {
    type Dispatcher = RouterDispatcher<S>;

    fn make(&self) -> RouterDispatcher<S> {
        RouterDispatcher::new(Arc::clone(self))
    }
}

impl<S: 'static> Dispatcher<S> for RouterDispatcher<S> {
    fn on_open(&mut self, ctx: Ctx<'_, S>) -> Result<(), DispatchError> {
        self.table = (ctx.version, self.router.table(ctx.version));
        match &self.router.on_open {
            Some(handler) => handler(ctx),
            None => Ok(()),
        }
    }

    fn on_version(&mut self, ctx: Ctx<'_, S>) -> Result<(), DispatchError> {
        let table = self.router.table(ctx.version);
        self.table = (ctx.version, table);
        Ok(())
    }

    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<(), DispatchError> {
        let router = &*self.router;
        let table = &router.tables[self.table.1].1;

        let Some(index) = table.lookup(ctx.phase, id) else {
            if router.unknown == UnknownPolicy::Ignore {
                trace!(id, phase = ?ctx.phase, "ignoring unhandled packet");
                return Ok(());
            }
            // A packet nobody registered is the peer's doing -- an unsupported client or someone
            // probing -- so it is classified as one and does not page anyone.
            return Err(DispatchError::peer(
                "unknown_packet",
                anyhow!(
                    "unknown packet ID {id:#04x} received in phase {:?} at version {}",
                    ctx.phase,
                    ctx.version
                ),
            ));
        };

        // Tracing lives here rather than on the connection, because this is where the name is
        // known -- a connection has an ID and a payload and nothing else.
        let entry = &router.entries[index as usize];
        trace!(packet = entry.name, phase = ?ctx.phase, "dispatching packet");
        (entry.dispatch)(ctx, payload)
    }

    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<(), DispatchError> {
        match &self.router.tick {
            Some(handler) => handler(ctx),
            None => Ok(()),
        }
    }

    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<(), DispatchError> {
        match &self.router.on_error {
            Some(handler) => handler(ctx, error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{ConnectionHandle, Op};
    use crate::packet::Packet;
    use crate::phase::Phase;
    use crate::router::Router;
    use crate::version::versions;
    use crate::wire::{Options, Reader, WireResult, Writer};
    use bytes::BytesMut;
    use std::sync::Mutex;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// What the handlers recorded, in order.
    type Seen = Mutex<Vec<String>>;

    /// A packet whose ID moved between versions, so dispatching it is a version decision.
    struct Moved {
        text: String,
    }

    impl Packet for Moved {
        const NAME: &'static str = "Moved";
        const PHASE: Phase = Phase::Login;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V26_2, 0x05), (versions::V1_20_5, 0x02)];

        fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self {
                text: r.string("text", 16)?,
            })
        }

        fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            w.string("text", &self.text)
        }
    }

    fn router(unknown: UnknownPolicy) -> Arc<Router<Seen>> {
        Arc::new(
            Router::<Seen>::builder()
                .unknown(unknown)
                .on::<Moved>(|ctx, packet| {
                    ctx.state.lock().expect("not poisoned").push(packet.text);
                    Ok(())
                })
                .expect("registers")
                .build(),
        )
    }

    /// The payload a frame carries: the ID varint, then the packet's own fields.
    fn payload(id: i32, write: impl FnOnce(&mut Writer<'_>)) -> BytesMut {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        writer.var_int(id);
        write(&mut writer);
        buf
    }

    /// A handle and the queue behind it, standing in for the connection that would drain it.
    fn handle() -> (ConnectionHandle<Seen>, mpsc::UnboundedReceiver<Op<Seen>>) {
        ConnectionHandle::new(CancellationToken::new(), Options::default())
    }

    #[test]
    fn a_registered_packet_reaches_its_handler() {
        let state = Seen::default();
        let (handle, _ops) = handle();
        let mut dispatcher = router(UnknownPolicy::Reject).make();

        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        dispatcher.on_open(ctx).expect("opens");
        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        let frame = payload(0x05, |w| w.string("text", "hello").expect("writes"));
        dispatcher.on_frame(ctx, 0x05, &frame).expect("dispatches");

        assert_eq!(
            *state.lock().expect("not poisoned"),
            vec!["hello".to_owned()]
        );
    }

    #[test]
    fn opening_binds_the_table_to_the_version_the_connection_starts_at() {
        // A client knows its version before it says anything, so it never queues `SetVersion`. If
        // opening did not bind the table, the whole connection would dispatch against the floor --
        // where this packet does not exist at all.
        let state = Seen::default();
        let (handle, _ops) = handle();
        let mut dispatcher = router(UnknownPolicy::Reject).make();
        assert_eq!(dispatcher.table, (ProtocolVersion::UNKNOWN, 0));

        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        dispatcher.on_open(ctx).expect("opens");
        assert_eq!(dispatcher.table.0, versions::V26_2);

        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        let frame = payload(0x05, |w| w.string("text", "bound").expect("writes"));
        dispatcher.on_frame(ctx, 0x05, &frame).expect("dispatches");
        assert_eq!(
            *state.lock().expect("not poisoned"),
            vec!["bound".to_owned()]
        );
    }

    #[test]
    fn a_version_change_rebinds_the_table() {
        let state = Seen::default();
        let (handle, _ops) = handle();
        let mut dispatcher = router(UnknownPolicy::Reject).make();

        // The same packet, under the ID its older version gives it.
        let ctx = Ctx::new(&state, Phase::Login, versions::V1_20_5, &handle);
        dispatcher.on_version(ctx).expect("rebinds");
        let ctx = Ctx::new(&state, Phase::Login, versions::V1_20_5, &handle);
        let frame = payload(0x02, |w| w.string("text", "older").expect("writes"));
        dispatcher.on_frame(ctx, 0x02, &frame).expect("dispatches");

        // And the newer ID is not in that table, which is the point of holding more than one.
        let ctx = Ctx::new(&state, Phase::Login, versions::V1_20_5, &handle);
        let frame = payload(0x05, |w| w.string("text", "newer").expect("writes"));
        assert!(dispatcher.on_frame(ctx, 0x05, &frame).is_err());
    }

    #[test]
    fn a_packet_nobody_registered_is_the_peers_doing() {
        // An unsupported client or someone probing: counted, logged at debug, nobody paged.
        let state = Seen::default();
        let (handle, _ops) = handle();
        let dispatcher = router(UnknownPolicy::Reject).make();

        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        let error = dispatcher
            .on_frame(ctx, 0x7F, &payload(0x7F, |_| {}))
            .expect_err("must fail the connection");
        assert!(error.is_peer_error());
        assert_eq!(error.label, "unknown_packet");
        assert!(error.to_string().contains("0x7f"), "{error}");
    }

    #[test]
    fn a_minimal_driver_can_ignore_what_it_does_not_route() {
        let state = Seen::default();
        let (handle, _ops) = handle();
        let dispatcher = router(UnknownPolicy::Ignore).make();

        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        dispatcher
            .on_frame(ctx, 0x7F, &payload(0x7F, |_| {}))
            .expect("ignored, not failed");
        assert!(state.lock().expect("not poisoned").is_empty());
    }

    #[test]
    fn a_payload_the_packet_cannot_account_for_fails_the_dispatch() {
        // Either we are misreading the packet or the peer is smuggling data past us. Both are worth
        // failing on, and the failure names the packet rather than the byte.
        let state = Seen::default();
        let (handle, _ops) = handle();
        let mut dispatcher = router(UnknownPolicy::Reject).make();
        dispatcher
            .on_open(Ctx::new(&state, Phase::Login, versions::V26_2, &handle))
            .expect("opens");

        let mut frame = payload(0x05, |w| w.string("text", "hello").expect("writes"));
        frame.extend_from_slice(b"trailing");
        let ctx = Ctx::new(&state, Phase::Login, versions::V26_2, &handle);
        let error = dispatcher
            .on_frame(ctx, 0x05, &frame)
            .expect_err("must fail the connection");
        assert!(error.to_string().contains("Moved"), "{error}");
    }

    #[test]
    fn a_router_without_hooks_answers_them_all_with_nothing() {
        let state = Seen::default();
        let (handle, _ops) = handle();
        let mut dispatcher = RouterDispatcher::new(Arc::new(Router::<Seen>::builder().build()));

        dispatcher
            .on_open(Ctx::new(&state, Phase::Login, versions::V26_2, &handle))
            .expect("nothing to do");
        dispatcher
            .on_tick(Ctx::new(&state, Phase::Login, versions::V26_2, &handle))
            .expect("nothing to do");
        let mut error = ConnectionError::shutdown();
        dispatcher
            .on_error(
                Ctx::new(&state, Phase::Login, versions::V26_2, &handle),
                &mut error,
            )
            .expect("nothing to do");
    }

    #[test]
    fn the_hooks_a_router_does_have_are_the_ones_it_runs() {
        let state = Seen::default();
        let (handle, _ops) = handle();
        let router = Arc::new(
            Router::<Seen>::builder()
                .on_open(|ctx| {
                    ctx.state
                        .lock()
                        .expect("not poisoned")
                        .push("open".to_owned());
                    Ok(())
                })
                .on_tick(|ctx| {
                    ctx.state
                        .lock()
                        .expect("not poisoned")
                        .push("tick".to_owned());
                    Ok(())
                })
                .on_error(|ctx, error| {
                    ctx.state
                        .lock()
                        .expect("not poisoned")
                        .push(error.reason().to_owned());
                    Ok(())
                })
                .build(),
        );
        let mut dispatcher = router.make();

        dispatcher
            .on_open(Ctx::new(&state, Phase::Login, versions::V26_2, &handle))
            .expect("opens");
        dispatcher
            .on_tick(Ctx::new(&state, Phase::Login, versions::V26_2, &handle))
            .expect("ticks");
        let mut error = ConnectionError::timeout();
        dispatcher
            .on_error(
                Ctx::new(&state, Phase::Login, versions::V26_2, &handle),
                &mut error,
            )
            .expect("reports");

        assert_eq!(
            *state.lock().expect("not poisoned"),
            vec![
                "open".to_owned(),
                "tick".to_owned(),
                "peer-timeout".to_owned()
            ],
        );
    }

    #[test]
    fn a_dispatcher_is_cheap_to_clone_and_says_what_it_routes_for() {
        let dispatcher = router(UnknownPolicy::Reject).make();
        let clone = dispatcher.clone();
        assert_eq!(clone.table, dispatcher.table);
        assert!(
            format!("{dispatcher:?}").contains("packets: 1"),
            "{dispatcher:?}",
        );
    }
}
