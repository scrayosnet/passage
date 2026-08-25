//! The sync-or-async return type for handlers, and how asynchronous handlers change state.

use crate::error::{Error, Result};
use futures::future::BoxFuture;

/// A result that may or may not require awaiting.
///
/// Most protocol handlers are pure state transitions: check a field, queue a packet, flip a phase.
/// Making them `async fn` forces every one of them into a boxed state machine that is polled once
/// and completed -- an allocation and a virtual call per packet for no benefit. `Flow` keeps the
/// common case free and pays only where a handler genuinely has to wait (an adapter call, a DNS
/// lookup, a gRPC round trip).
///
/// It is the same shape as `futures::future::Either<Ready<T>, BoxFuture<T>>`, but as a named type
/// the driver can match on, which is what lets it enforce ordering (see
/// [`Driver`](crate::driver::Driver)).
#[must_use = "a Flow does nothing unless it is returned to the driver"]
pub enum Flow<T> {
    /// The handler completed synchronously.
    Ready(T),

    /// The handler needs to be polled to completion.
    ///
    /// The future is `'static`: it cannot borrow connection state. That is deliberate -- an async
    /// handler has to decide explicitly what it takes with it (a cloned
    /// [`ConnHandle`](crate::conn::ConnHandle), the fields it needs) instead of holding a lock
    /// across an await point. What it hands *back* is an [`Update`].
    Pending(BoxFuture<'static, T>),
}

impl<T> Flow<T> {
    /// Wraps a value that is already available.
    pub fn ready(value: T) -> Self {
        Flow::Ready(value)
    }

    /// Wraps a future to be driven by the driver.
    pub fn later(future: impl Future<Output = T> + Send + 'static) -> Self {
        Flow::Pending(Box::pin(future))
    }

    /// Whether this flow still has to be polled.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Flow::Pending(_))
    }
}

/// A deferred change to per-connection state.
///
/// This is how an asynchronous handler writes to state it is not allowed to borrow. The driver
/// applies the update with exclusive access, at a defined point: after the handler resolves and
/// before the next packet is read. So an async handler gets the same "no locks, no interleaving"
/// guarantee as a synchronous one, without an `Arc<Mutex<S>>` whose lock could be held across an
/// await point.
pub struct Update<S> {
    change: Option<Change<S>>,
}

/// The boxed state change carried by an [`Update`].
type Change<S> = Box<dyn FnOnce(&mut S) + Send>;

impl<S> Update<S> {
    /// No change.
    #[must_use]
    pub fn none() -> Self {
        Self { change: None }
    }

    /// Applies `change` to the connection state once the driver regains control.
    #[must_use]
    pub fn apply(change: impl FnOnce(&mut S) + Send + 'static) -> Self {
        Self {
            change: Some(Box::new(change)),
        }
    }

    pub(crate) fn run(self, state: &mut S) {
        if let Some(change) = self.change {
            change(state);
        }
    }
}

impl<S> Default for Update<S> {
    fn default() -> Self {
        Self::none()
    }
}

impl<S> std::fmt::Debug for Update<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Update")
            .field("changes_state", &self.change.is_some())
            .finish()
    }
}

/// What a handler returns.
pub type Outcome<S> = Flow<Result<Update<S>>>;

impl<S> Flow<Result<Update<S>>> {
    /// Done, nothing more to do.
    pub fn done() -> Self {
        Flow::Ready(Ok(Update::none()))
    }

    /// Done, with a change to connection state.
    pub fn update(change: impl FnOnce(&mut S) + Send + 'static) -> Self {
        Flow::Ready(Ok(Update::apply(change)))
    }

    /// Failed.
    pub fn fail(error: impl Into<Error>) -> Self {
        Flow::Ready(Err(error.into()))
    }

    /// Turns a synchronous result into a flow, so handlers can use `?`-style helpers.
    pub fn from_result(result: Result<()>) -> Self {
        match result {
            Ok(()) => Flow::done(),
            Err(err) => Flow::fail(err),
        }
    }
}
