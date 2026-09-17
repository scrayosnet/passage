use std::sync::Arc;
use anyhow::bail;
use tracing::trace;
use crate::connection::{ConnectionError, Ctx, DispatchError, Dispatcher, MakeDispatcher};
use crate::router::{Router, UnknownPolicy};
use crate::version::ProtocolVersion;

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
        Self { router: router.into(), table: (ProtocolVersion::UNKNOWN, 0) }
    }
}

impl<S: 'static> MakeDispatcher<S> for Arc<Router<S>> {
    type Dispatcher = RouterDispatcher<S>;

    fn make(&self) -> RouterDispatcher<S> {
        RouterDispatcher::new(Arc::clone(self))
    }
}

impl<S: 'static> Dispatcher<S> for RouterDispatcher<S> {
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
            // While not ideal, we send an 'anyhow' error from the library code here. This makes the
            // error handling easier.
            bail!("unknown packet ID {id} received in phase {:?} at version {:?}", ctx.phase, ctx.version);
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
