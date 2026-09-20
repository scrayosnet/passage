use crate::common::ProtocolVersion;
use crate::connection::{ConnRef, ConnectionError, DispatchError, Dispatcher, MakeDispatcher};
use crate::router::{Router, UnknownPolicy};
use anyhow::anyhow;
use bytes::Bytes;
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::trace;

/// Creates a future that directly resolves to the result.
fn ready<'a>(result: Result<(), DispatchError>) -> BoxFuture<'a, Result<(), DispatchError>> {
    Box::pin(std::future::ready(result))
}

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
    fn on_open(&mut self, conn: ConnRef<'_, S>) -> Result<(), DispatchError> {
        let version = conn.version();
        self.table = (version, self.router.table(version));
        match &self.router.on_open {
            Some(handler) => handler(conn),
            None => Ok(()),
        }
    }

    fn on_version(&mut self, conn: ConnRef<'_, S>) -> Result<(), DispatchError> {
        let version = conn.version();
        self.table = (version, self.router.table(version));
        Ok(())
    }

    fn on_frame<'a>(
        &self,
        conn: ConnRef<'a, S>,
        id: i32,
        payload: Bytes,
    ) -> BoxFuture<'a, Result<(), DispatchError>> {
        let router = &*self.router;
        let table = &router.tables[self.table.1].1;
        let (phase, version) = conn.with(|c| (c.phase(), c.version()));

        let Some(index) = table.lookup(phase, id) else {
            if router.unknown == UnknownPolicy::Ignore {
                trace!(id, ?phase, "ignoring unhandled packet");
                return ready(Ok(()));
            }
            return ready(Err(DispatchError::peer(
                "unknown_packet",
                anyhow!(
                    "unknown packet ID {id:#04x} received in phase {phase:?} at version {version}"
                ),
            )));
        };

        let entry = &router.entries[index as usize];
        trace!(packet = entry.name, ?phase, "dispatching packet");
        (entry.dispatch)(conn, payload)
    }

    fn on_tick<'a>(&self, conn: ConnRef<'a, S>) -> BoxFuture<'a, Result<(), DispatchError>> {
        match &self.router.tick {
            Some(handler) => handler(conn),
            None => ready(Ok(())),
        }
    }

    fn on_error(
        &self,
        conn: ConnRef<'_, S>,
        error: &mut ConnectionError,
    ) -> Result<(), DispatchError> {
        match &self.router.on_error {
            Some(handler) => handler(conn, error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Phase;
    use crate::common::versions;
    use crate::connection::ConnCell;
    use crate::packet::packet::Packet;
    use crate::router::Router;
    use crate::wire::{Options, Reader, WireResult, Writer};
    use bytes::BytesMut;
    use bytestring::ByteString;
    use std::sync::Mutex;

    /// What the handlers recorded, in order.
    type Seen = Mutex<Vec<String>>;

    /// A packet whose ID moved between versions, so dispatching it is a version decision.
    struct Moved {
        text: ByteString,
    }

    impl Packet for Moved {
        const NAME: &'static str = "Moved";
        const PHASE: Phase = Phase::Login;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V26_1, 0x05), (versions::V1_20_5, 0x02)];

        fn decode(r: &mut Reader, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self {
                text: r.string("text", 16)?,
            })
        }

        fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            w.string("text", &self.text)
        }
    }

    /// A handler is a plain `async fn`. `P` is inferred from its signature, so the registration
    /// below needs no turbofish and no `Box::pin`.
    async fn note(conn: ConnRef<'_, Seen>, packet: Moved) -> Result<(), DispatchError> {
        conn.with(|c| {
            c.state
                .lock()
                .expect("not poisoned")
                .push(packet.text.into())
        });
        Ok(())
    }

    fn router(unknown: UnknownPolicy) -> Arc<Router<Seen>> {
        Arc::new(
            Router::<Seen>::builder()
                .unknown(unknown)
                .on(note)
                .expect("registers")
                .build(),
        )
    }

    /// The payload a frame carries: the ID varint, then the packet's own fields. Frozen, because
    /// that is the shape a decoded frame arrives in -- and what lets a packet keep a slice of it.
    fn payload(id: i32, write: impl FnOnce(&mut Writer<'_>)) -> Bytes {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        writer.var_int(id);
        write(&mut writer);
        buf.freeze()
    }

    /// A connection standing in for the one that would be driving these handlers.
    fn cell(version: ProtocolVersion) -> ConnCell<Seen> {
        ConnCell::new(Seen::default(), version, Phase::Login, Options::default())
    }

    /// What the handlers recorded on `cell`, in order.
    fn seen(cell: &ConnCell<Seen>) -> Vec<String> {
        cell.as_ref()
            .with(|c| c.state.lock().expect("not poisoned").clone())
    }

    #[tokio::test]
    async fn a_registered_packet_reaches_its_handler() {
        let cell = cell(versions::V26_1);
        let mut dispatcher = router(UnknownPolicy::Reject).make();

        dispatcher.on_open(cell.as_ref()).expect("opens");
        let frame = payload(0x05, |w| w.string("text", "hello").expect("writes"));
        dispatcher
            .on_frame(cell.as_ref(), 0x05, frame.clone())
            .await
            .expect("dispatches");

        assert_eq!(seen(&cell), vec!["hello".to_owned()]);
    }

    #[tokio::test]
    async fn opening_binds_the_table_to_the_version_the_connection_starts_at() {
        // A client knows its version before it says anything, so nothing ever changes it. If
        // opening did not bind the table, the whole connection would dispatch against the floor --
        // where this packet does not exist at all.
        let cell = cell(versions::V26_1);
        let mut dispatcher = router(UnknownPolicy::Reject).make();
        assert_eq!(dispatcher.table, (ProtocolVersion::UNKNOWN, 0));

        dispatcher.on_open(cell.as_ref()).expect("opens");
        assert_eq!(dispatcher.table.0, versions::V26_1);

        let frame = payload(0x05, |w| w.string("text", "bound").expect("writes"));
        dispatcher
            .on_frame(cell.as_ref(), 0x05, frame.clone())
            .await
            .expect("dispatches");
        assert_eq!(seen(&cell), vec!["bound".to_owned()]);
    }

    #[tokio::test]
    async fn a_version_change_rebinds_the_table() {
        let cell = cell(versions::V1_20_5);
        let mut dispatcher = router(UnknownPolicy::Reject).make();

        // The same packet, under the ID its older version gives it.
        dispatcher.on_version(cell.as_ref()).expect("rebinds");
        let frame = payload(0x02, |w| w.string("text", "older").expect("writes"));
        dispatcher
            .on_frame(cell.as_ref(), 0x02, frame.clone())
            .await
            .expect("dispatches");

        // And the newer ID is not in that table, which is the point of holding more than one.
        let frame = payload(0x05, |w| w.string("text", "newer").expect("writes"));
        assert!(
            dispatcher
                .on_frame(cell.as_ref(), 0x05, frame.clone())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_handler_that_suspends_does_not_hold_the_connection() {
        // The reason `with` takes a synchronous closure: a handler that awaits between two reads
        // of the state cannot be holding the lock while it waits, so the connection stays usable
        // by whatever else the loop is driving.
        async fn slow(conn: ConnRef<'_, Seen>, packet: Moved) -> Result<(), DispatchError> {
            conn.with(|c| {
                c.state
                    .lock()
                    .expect("not poisoned")
                    .push("before".to_owned())
            });
            tokio::task::yield_now().await;
            conn.with(|c| {
                c.state
                    .lock()
                    .expect("not poisoned")
                    .push(packet.text.into())
            });
            Ok(())
        }

        let cell = cell(versions::V26_1);
        let router = Arc::new(
            Router::<Seen>::builder()
                .on(slow)
                .expect("registers")
                .build(),
        );
        let mut dispatcher = router.make();
        dispatcher.on_open(cell.as_ref()).expect("opens");

        let frame = payload(0x05, |w| w.string("text", "after").expect("writes"));
        let mut handled = dispatcher.on_frame(cell.as_ref(), 0x05, frame.clone());

        // Park it at the await, then reach the connection from outside the handler.
        let polled = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(std::pin::Pin::new(&mut handled).poll(cx))
        })
        .await;
        assert!(polled.is_pending(), "the handler parked");
        assert_eq!(seen(&cell), vec!["before".to_owned()], "and let go");

        handled.await.expect("resumes");
        assert_eq!(seen(&cell), vec!["before".to_owned(), "after".to_owned()]);
    }

    #[tokio::test]
    async fn a_packet_nobody_registered_is_the_peers_doing() {
        // An unsupported client or someone probing: counted, logged at debug, nobody paged.
        let cell = cell(versions::V26_1);
        let dispatcher = router(UnknownPolicy::Reject).make();

        let error = dispatcher
            .on_frame(cell.as_ref(), 0x7F, payload(0x7F, |_| {}))
            .await
            .expect_err("must fail the connection");
        assert!(error.is_peer_error());
        assert_eq!(error.label, "unknown_packet");
        assert!(error.to_string().contains("0x7f"), "{error}");
    }

    #[tokio::test]
    async fn a_minimal_driver_can_ignore_what_it_does_not_route() {
        let cell = cell(versions::V26_1);
        let dispatcher = router(UnknownPolicy::Ignore).make();

        dispatcher
            .on_frame(cell.as_ref(), 0x7F, payload(0x7F, |_| {}))
            .await
            .expect("ignored, not failed");
        assert!(seen(&cell).is_empty());
    }

    #[tokio::test]
    async fn a_payload_the_packet_cannot_account_for_fails_the_dispatch() {
        // Either we are misreading the packet or the peer is smuggling data past us. Both are worth
        // failing on, and the failure names the packet rather than the byte.
        let cell = cell(versions::V26_1);
        let mut dispatcher = router(UnknownPolicy::Reject).make();
        dispatcher.on_open(cell.as_ref()).expect("opens");

        let mut frame = BytesMut::from(payload(0x05, |w| {
            w.string("text", "hello").expect("writes")
        }));
        frame.extend_from_slice(b"trailing");
        let frame = frame.freeze();
        let error = dispatcher
            .on_frame(cell.as_ref(), 0x05, frame.clone())
            .await
            .expect_err("must fail the connection");
        assert!(error.to_string().contains("Moved"), "{error}");
    }

    #[tokio::test]
    async fn a_router_without_hooks_answers_them_all_with_nothing() {
        let cell = cell(versions::V26_1);
        let mut dispatcher = RouterDispatcher::new(Arc::new(Router::<Seen>::builder().build()));

        dispatcher.on_open(cell.as_ref()).expect("nothing to do");
        dispatcher
            .on_tick(cell.as_ref())
            .await
            .expect("nothing to do");
        let mut error = ConnectionError::shutdown();
        dispatcher
            .on_error(cell.as_ref(), &mut error)
            .expect("nothing to do");
    }

    #[tokio::test]
    async fn the_hooks_a_router_does_have_are_the_ones_it_runs() {
        async fn ticked(conn: ConnRef<'_, Seen>) -> Result<(), DispatchError> {
            conn.with(|c| {
                c.state
                    .lock()
                    .expect("not poisoned")
                    .push("tick".to_owned())
            });
            Ok(())
        }

        let cell = cell(versions::V26_1);
        let router = Arc::new(
            Router::<Seen>::builder()
                .on_open(|conn: ConnRef<'_, Seen>| {
                    conn.with(|c| {
                        c.state
                            .lock()
                            .expect("not poisoned")
                            .push("open".to_owned())
                    });
                    Ok(())
                })
                .on_tick(ticked)
                .on_error(|conn: ConnRef<'_, Seen>, error: &mut ConnectionError| {
                    conn.with(|c| {
                        c.state
                            .lock()
                            .expect("not poisoned")
                            .push(error.reason().to_owned());
                    });
                    Ok(())
                })
                .build(),
        );
        let mut dispatcher = router.make();

        dispatcher.on_open(cell.as_ref()).expect("opens");
        dispatcher.on_tick(cell.as_ref()).await.expect("ticks");
        let mut error = ConnectionError::timeout();
        dispatcher
            .on_error(cell.as_ref(), &mut error)
            .expect("reports");

        assert_eq!(
            seen(&cell),
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
