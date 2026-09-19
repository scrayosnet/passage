use crate::connection::{ConnectionError, Ctx};
use std::fmt;

/// Who caused a handler failure.
///
/// The driver cannot know whether a failed authentication is the peer's fault, ours, or a
/// dependency's, so the handler that raised it says. This is what decides the log level and whether
/// a failure is worth reporting: see [`ConnectionError::is_peer_error`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Class {
    /// The peer sent something illegal or stopped playing along. Expected in the wild (scanners,
    /// mods, bots, timeouts): count it, log it at `debug`, never page anyone.
    Peer,

    /// A bug on our side, or a dependency failing. Log it at `warn`/`error` and report it. The
    /// default, because an unclassified failure is one nobody has thought about yet.
    #[default]
    Internal,
}

/// An error raised by a [`Dispatcher`]'s handlers.
///
/// The source is an [`anyhow::Error`] because the driver cannot list what a custom handler will
/// fail with. What it *can* ask for is the two things telemetry needs and a handler always knows:
/// who is to blame ([`class`](DispatchError::class)) and a stable metric label
/// ([`label`](DispatchError::label)). A plain `?` on an [`anyhow::Error`] fills both with the
/// conservative default, so handlers that do not care pay nothing.
#[derive(Debug)]
pub struct DispatchError {
    /// Who is to blame.
    pub class: Class,

    /// A stable, low-cardinality metric label. Never peer-controlled.
    pub label: &'static str,

    /// The underlying error.
    pub source: anyhow::Error,
}

/// The label a [`DispatchError`] carries when a handler did not choose one.
const DEFAULT_LABEL: &str = "dispatch";

impl DispatchError {
    /// Raises a handler error the peer is to blame for: a rejection, a timeout, a failed check.
    #[must_use]
    pub fn peer(label: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self {
            class: Class::Peer,
            label,
            source: source.into(),
        }
    }

    /// Raises a handler error we are to blame for: a bug, a misconfiguration, a failing dependency.
    #[must_use]
    pub fn internal(label: &'static str, source: impl Into<anyhow::Error>) -> Self {
        Self {
            class: Class::Internal,
            label,
            source: source.into(),
        }
    }

    /// Whether the peer is to blame for this error.
    #[must_use]
    pub fn is_peer_error(&self) -> bool {
        self.class == Class::Peer
    }
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.source, f)
    }
}

impl std::error::Error for DispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// So that a handler can `?` an [`anyhow::Error`] without classifying it. An unclassified failure is
/// [`Class::Internal`], because assuming the peer's fault would hide our own bugs.
impl From<anyhow::Error> for DispatchError {
    fn from(source: anyhow::Error) -> Self {
        Self {
            class: Class::Internal,
            label: DEFAULT_LABEL,
            source,
        }
    }
}

/// So that a handler can `?` the queue operations on its own [`Ctx`] -- `ctx.handle.close()?` and
/// friends return a [`ConnectionError`]. The classification the connection already made is carried
/// across rather than flattened to the default.
impl From<ConnectionError> for DispatchError {
    fn from(error: ConnectionError) -> Self {
        Self {
            class: if error.is_peer_error() {
                Class::Peer
            } else {
                Class::Internal
            },
            label: error.reason(),
            source: anyhow::Error::new(error),
        }
    }
}

/// The dispatch result type, defaulting to [`DispatchError`]. Private, so that the crate has one
/// exported `Result` alias ([`ConnectionError`]'s) rather than two that shadow each other.
type Result<T, E = DispatchError> = std::result::Result<T, E>;

/// A [`Dispatcher`] is used by the connection to handle incoming packets. It is implemented for
/// [`Box`] and [`Option`].
///
/// Every method has a default that does nothing, so an implementation only writes the hooks it
/// cares about.
pub trait Dispatcher<S> {
    /// Called once, before the connection reads or writes anything.
    ///
    /// # Errors
    ///
    /// Returns whatever the implementation raises. An error here fails the connection before the
    /// first frame, and [`on_error`](Dispatcher::on_error) still gets the last word.
    fn on_open(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Called when the connection changes the protocol version. This can be used to update internal
    /// dispatch tables.
    ///
    /// # Errors
    ///
    /// Returns whatever the implementation raises while rebinding. An error here fails the
    /// connection.
    fn on_version(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Handles an incoming packet. `payload` leads with the ID varint the frame was routed by.
    ///
    /// # Errors
    ///
    /// Returns whatever the handler raises. An error here fails the connection, and
    /// [`on_error`](Dispatcher::on_error) gets the last word before the socket goes away.
    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let _ = (ctx, id, payload);
        Ok(())
    }

    /// Handles a tick event.
    ///
    /// # Errors
    ///
    /// Returns whatever the tick handler raises. An error here fails the connection.
    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Handles a connection error. It is called before the connection is closed and should be used
    /// to send custom disconnect packets to the peer.
    ///
    /// It is a last word, not a veto: whether the connection ends was decided before it was called,
    /// and the error it returns is logged rather than reported. It may rewrite `error`, which is
    /// what the connection then reports. Check
    /// [`ConnectionError::can_reply`] before composing a message: after a hangup there is nobody
    /// left to read it.
    ///
    /// # Errors
    ///
    /// Returns whatever the hook raises. The failure is logged and does not replace the ending the
    /// connection already had.
    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        let _ = (ctx, error);
        Ok(())
    }
}

impl<S> Dispatcher<S> for () {}

impl<S, D: Dispatcher<S> + ?Sized> Dispatcher<S> for Box<D> {
    fn on_open(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        (**self).on_open(ctx)
    }

    fn on_version(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        (**self).on_version(ctx)
    }

    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        (**self).on_frame(ctx, id, payload)
    }

    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        (**self).on_tick(ctx)
    }

    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        (**self).on_error(ctx, error)
    }
}

// There is deliberately no impl for `Arc<D>`. `on_version` takes `&mut self`, so a shared dispatcher
// could not rebind its table -- it would silently keep serving every connection from the table it
// started on, which is exactly the bug the connection calls `on_version` to prevent.

impl<S, D: Dispatcher<S>> Dispatcher<S> for Option<D> {
    fn on_open(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_open(ctx)
    }

    fn on_version(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_version(ctx)
    }

    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_frame(ctx, id, payload)
    }

    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_tick(ctx)
    }

    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_error(ctx, error)
    }
}

/// [`MakeDispatcher`] is a builder for [`Dispatcher`]s. In general, a connection [`Dispatcher`] is
/// stateful. This builder allows the driver to create a new dispatcher for each request.
pub trait MakeDispatcher<S>: Send + 'static {
    /// The dispatcher this produces.
    type Dispatcher: Dispatcher<S> + Send + 'static;

    /// Makes one, for one connection.
    fn make(&self) -> Self::Dispatcher;
}

impl<S: 'static> MakeDispatcher<S> for () {
    type Dispatcher = ();

    fn make(&self) {}
}

/// A [`MakeDispatcherFn`] builds a dispatcher from a closure. See [`make_with`].
pub struct MakeDispatcherFn<F>(F);

impl<S, D, F> MakeDispatcher<S> for MakeDispatcherFn<F>
where
    F: Fn() -> D + Send + 'static,
    D: Dispatcher<S> + Send + 'static,
{
    type Dispatcher = D;

    fn make(&self) -> D {
        self.0()
    }
}

/// Makes a dispatcher from a closure. Useful for stateless dispatchers.
pub fn make_with<F>(make: F) -> MakeDispatcherFn<F> {
    MakeDispatcherFn(make)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Phase;
    use crate::common::{ProtocolVersion, versions};
    use crate::connection::ConnectionHandle;
    use crate::wire::Options;
    use anyhow::anyhow;
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;

    /// Which hooks a dispatcher was asked to run, in order.
    #[derive(Default)]
    struct Recorder {
        seen: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Dispatcher<()> for Recorder {
        fn on_open(&mut self, _ctx: Ctx<'_, ()>) -> Result<()> {
            self.seen.lock().expect("not poisoned").push("open");
            Ok(())
        }

        fn on_version(&mut self, _ctx: Ctx<'_, ()>) -> Result<()> {
            self.seen.lock().expect("not poisoned").push("version");
            Ok(())
        }

        fn on_frame(&self, _ctx: Ctx<'_, ()>, _id: i32, _payload: &[u8]) -> Result<()> {
            self.seen.lock().expect("not poisoned").push("frame");
            Ok(())
        }

        fn on_tick(&self, _ctx: Ctx<'_, ()>) -> Result<()> {
            self.seen.lock().expect("not poisoned").push("tick");
            Ok(())
        }

        fn on_error(&self, _ctx: Ctx<'_, ()>, _error: &mut ConnectionError) -> Result<()> {
            self.seen.lock().expect("not poisoned").push("error");
            Ok(())
        }
    }

    /// Runs every hook on `dispatcher`, so a test only has to say what it expects to be recorded.
    fn run_every_hook(mut dispatcher: impl Dispatcher<()>) {
        let state = ();
        let (handle, _ops) =
            ConnectionHandle::<()>::new(CancellationToken::new(), Options::default());
        let ctx = || Ctx::new(&state, Phase::Login, versions::V26_1, &handle);

        dispatcher.on_open(ctx()).expect("opens");
        dispatcher.on_version(ctx()).expect("rebinds");
        dispatcher.on_frame(ctx(), 0x00, &[]).expect("dispatches");
        dispatcher.on_tick(ctx()).expect("ticks");
        let mut error = ConnectionError::shutdown();
        dispatcher.on_error(ctx(), &mut error).expect("reports");
    }

    #[test]
    fn a_dispatcher_that_implements_nothing_does_nothing() {
        // Every method has a default that returns `Ok`, so an implementation only writes the hooks
        // it cares about -- and the call itself is the assertion.
        run_every_hook(());
    }

    #[test]
    fn a_boxed_dispatcher_forwards_every_hook() {
        // Which dispatcher runs is a value, not a type -- and a `Box` that dropped a hook would be
        // a silent bug in exactly the case the hook exists to prevent.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let dispatcher: Box<dyn Dispatcher<()>> = Box::new(Recorder {
            seen: Arc::clone(&seen),
        });
        run_every_hook(dispatcher);
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            vec!["open", "version", "frame", "tick", "error"],
        );
    }

    #[test]
    fn an_optional_dispatcher_forwards_every_hook_it_has() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        run_every_hook(Some(Recorder {
            seen: Arc::clone(&seen),
        }));
        assert_eq!(
            *seen.lock().expect("not poisoned"),
            vec!["open", "version", "frame", "tick", "error"],
        );

        run_every_hook(None::<Recorder>);
        assert_eq!(
            seen.lock().expect("not poisoned").len(),
            5,
            "nothing was added"
        );
    }

    #[test]
    fn an_unclassified_failure_is_ours() {
        // A plain `?` on an `anyhow::Error` fills both fields with the conservative default, so
        // handlers that do not care pay nothing -- and a failure nobody thought about is not
        // quietly blamed on the peer.
        let error = DispatchError::from(anyhow!("something went wrong"));
        assert_eq!(error.class, Class::Internal);
        assert_eq!(error.label, DEFAULT_LABEL);
        assert!(!error.is_peer_error());
        assert_eq!(error.to_string(), "something went wrong");
        assert_eq!(Class::default(), Class::Internal);
    }

    #[test]
    fn a_handler_says_who_is_to_blame_because_only_it_knows() {
        let peer = DispatchError::peer("bad_client", anyhow!("unsupported version"));
        assert!(peer.is_peer_error());
        assert_eq!(peer.label, "bad_client");

        let ours = DispatchError::internal("upstream", anyhow!("the database is down"));
        assert!(!ours.is_peer_error());
        assert_eq!(ours.label, "upstream");

        // The cause survives as a source, so a report can still unwrap the whole chain.
        let source = std::error::Error::source(&ours).expect("a cause");
        assert_eq!(source.to_string(), "the database is down");
    }

    #[test]
    fn a_dispatcher_factory_builds_one_per_connection() {
        // Dispatchers are generally stateful, so the server asks for a new one per socket.
        let made = Arc::new(Mutex::new(0));
        let counter = Arc::clone(&made);
        let factory = make_with(move || {
            *counter.lock().expect("not poisoned") += 1;
            Recorder::default()
        });

        let _: Recorder = MakeDispatcher::<()>::make(&factory);
        let _: Recorder = MakeDispatcher::<()>::make(&factory);
        assert_eq!(*made.lock().expect("not poisoned"), 2);

        // And a connection that handles nothing needs no dispatcher at all.
        MakeDispatcher::<()>::make(&());
    }

    #[test]
    fn a_version_the_dispatcher_never_hears_about_is_the_bug_on_open_prevents() {
        // Documented here because it is the whole reason `on_open` takes `&mut self`: a dispatcher
        // that binds a version-dependent table only on `on_version` never binds it for a client.
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = Recorder {
            seen: Arc::clone(&seen),
        };
        let state = ();
        let (handle, _ops) =
            ConnectionHandle::<()>::new(CancellationToken::new(), Options::default());
        dispatcher
            .on_open(Ctx::new(
                &state,
                Phase::Handshake,
                ProtocolVersion::UNKNOWN,
                &handle,
            ))
            .expect("opens");
        assert_eq!(*seen.lock().expect("not poisoned"), vec!["open"]);
    }
}
