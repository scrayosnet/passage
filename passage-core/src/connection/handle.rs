use crate::codec::{Cipher, Frame};
use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::DispatchError;
use crate::connection::error::Result;
use crate::packet::packet::Packet;
use crate::wire::Options;
use std::sync::{Mutex, TryLockError};
use std::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::Span;

/// An operation that should be applied to the outgoing socket in order. This only applies to socket
/// operations which are order-sensitive (e.g., send before encrypt).
pub enum Out {
    /// An encoded packet.
    Frame(Frame),

    /// Enable encryption.
    Cipher(Box<dyn Cipher>),
}

impl std::fmt::Debug for Out {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Out::Frame(frame) => write!(f, "Frame({})", frame.name),
            Out::Cipher(_) => f.write_str("Cipher"),
        }
    }
}

/// The connection handle. It is used by the dispatch handlers to interact with the connection,
/// representing the partial, mutable state of the connection.
///
/// A single handle is shared between all dispatch handlers as a [`ConnRef`] (i.e., a mutex reference
/// of this). This represents the actual connection state.
pub struct Conn<S> {
    /// The current per-connection state.
    pub state: S,

    /// The span the connection itself runs in, which every handler span is a descendant of. A
    /// handler that hands a trace context to somebody else -- a backend the player is transferred
    /// to, say -- wants this one rather than its own: what follows belongs to the connection, not
    /// to the packet that happened to be in flight when it was handed over.
    span: Span,

    /// Queued outgoing operations. They are applied concurrently to the handlers.
    out: Vec<Out>,

    /// The current protocol version. Packets are encoded against this as of the moment they are
    /// queued, which is why there is no such thing as a stale encoding.
    version: ProtocolVersion,

    /// The current phase, which decides how incoming frames are routed.
    phase: Phase,

    /// The wire options used for encoding.
    options: Options,

    /// Whether the peer is expected to stay quiet. A frame arriving while this is set is a protocol
    /// break rather than input to be handled later. Open to begin with, because a server reads the
    /// handshake before any handler of its own has run.
    gated: bool,

    /// Whether the connection should end once the queue has been cleared.
    closing: bool,

    /// When the connection gives up on its own, if it has a deadline at all. Re-read every round.
    deadline: Option<Instant>,

    /// The token that ends the connection from outside.
    shutdown: CancellationToken,

    /// What the connection is ending for, if it is ending for a failure. It is what the
    /// [`Outcome`](crate::connection::Outcome) reports.
    error: Option<DispatchError>,
}

impl<S> Conn<S> {
    /// The span the connection runs in: the parent of every handler span, and what a handler
    /// propagates when it hands the trace to another service.
    #[must_use]
    pub fn span(&self) -> &Span {
        &self.span
    }

    /// The protocol version the connection is in.
    #[must_use]
    pub fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// The phase the connection is in.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The wire options this connection encodes with.
    #[must_use]
    pub fn options(&self) -> Options {
        self.options
    }

    /// Sets the protocol version. This affects how packets are encoded from here on, and how
    /// incoming frames are routed.
    pub fn set_version(&mut self, version: ProtocolVersion) {
        self.version = version;
    }

    /// Sets the phase. This affects how packets are encoded from here on, and how
    /// incoming frames are routed.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    /// When the connection gives up on its own, if it has a deadline at all.
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Moves (or removes) the deadline. It is read again every round, so it takes effect on the
    /// next one.
    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    /// Sets the deadline to `after` from now.
    pub fn expire_in(&mut self, after: Duration) {
        self.deadline = Some(Instant::now() + after);
    }

    /// The token the connection ends on. A handler may await it to answer a shutdown itself.
    #[must_use]
    pub fn shutdown(&self) -> &CancellationToken {
        &self.shutdown
    }

    /// Replaces the shutdown token, so the connection ends on `shutdown` from here on.
    pub fn set_shutdown(&mut self, shutdown: CancellationToken) {
        self.shutdown = shutdown;
    }

    /// Replaces the shutdown token with a fresh one. A handler answering a cancellation needs this,
    /// or the same cancellation cuts off what it writes.
    pub fn detach(&mut self) {
        self.shutdown = CancellationToken::new();
    }

    /// Encodes a packet against the current version and queues it for the wire.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Codec`](crate::connection::ConnectionError::Codec) if the packet
    /// does not exist in the connection's version, or if its own encoder fails.
    pub fn send<P: Packet>(&mut self, packet: P) -> Result<()> {
        let frame = Frame::of(&packet, self.version, self.options)?;
        self.out.push(Out::Frame(frame));
        Ok(())
    }

    /// Enables encryption from this point on the wire.
    pub fn encrypt(&mut self, cipher: Box<dyn Cipher>) {
        self.out.push(Out::Cipher(cipher));
    }

    /// Requires the peer to stay quiet until [`release`](Conn::release) is called: work it is
    /// expected to wait for, such as an authentication round trip.
    ///
    /// The socket is still read, so a hangup is still noticed. A frame that arrives while the gate
    /// is shut ends the connection with
    /// [`ConnectionError::EarlyPacket`](crate::connection::ConnectionError::EarlyPacket).
    pub fn gate(&mut self) {
        self.gated = true;
    }

    /// Lets the peer speak again after [`gate`](Conn::gate) was called.
    pub fn release(&mut self) {
        self.gated = false;
    }

    /// Whether the peer is currently required to stay quiet.
    #[must_use]
    pub fn gated(&self) -> bool {
        self.gated
    }

    /// Ends the connection, once everything queued has been written.
    pub fn close(&mut self) {
        self.closing = true;
    }

    /// Ends the connection with a reason, once everything queued has been written. The first
    /// failure is the one reported; later ones are its wake and are dropped.
    pub fn fail(&mut self, error: DispatchError) {
        self.error.get_or_insert(error);
        self.closing = true;
    }

    /// Whether the connection is ending.
    #[must_use]
    pub fn closing(&self) -> bool {
        self.closing
    }

    /// Takes what the connection is ending for, if it is ending for a failure.
    pub(crate) fn take_error(&mut self) -> Option<DispatchError> {
        self.error.take()
    }

    /// Whether anything is waiting for the wire. The loop asks after polling its handlers, so that
    /// one which queued a packet and then went back to waiting does not hold it until the next
    /// event.
    pub(crate) fn queued(&self) -> bool {
        !self.out.is_empty()
    }

    /// Swaps the outbox to the loop, leaving `spare`'s allocation behind.
    pub(crate) fn swap_out(&mut self, spare: &mut Vec<Out>) {
        std::mem::swap(&mut self.out, spare);
    }
}

/// A cell to a [connection handle](Conn).
pub struct ConnCell<S> {
    inner: Mutex<Conn<S>>,
}

impl<S> ConnCell<S> {
    /// Creates a cell holding `state`, starting in `version` and `phase`.
    ///
    /// It starts with no deadline and a token nobody else holds; the connection arms both from its
    /// own configuration before it runs. The peer is free to speak: a handler shuts the gate when
    /// it wants it quiet.
    ///
    /// The span is taken from where this is built, which is inside the connection's own span: the
    /// cell is created by [`Connection::run`](crate::connection::Connection::run), and every
    /// handler future is polled from that same task.
    pub(crate) fn new(state: S, version: ProtocolVersion, phase: Phase, options: Options) -> Self {
        Self {
            inner: Mutex::new(Conn {
                state,
                span: Span::current(),
                out: Vec::new(),
                version,
                phase,
                options,
                gated: false,
                closing: false,
                deadline: None,
                shutdown: CancellationToken::new(),
                error: None,
            }),
        }
    }

    /// Takes the connection back once nothing can be holding it.
    pub(crate) fn into_inner(self) -> Conn<S> {
        // A poisoned lock means a handler panicked while holding it. The connection is ending
        // either way, and the state is what the outcome reports, so it is recovered rather than
        // re-raised.
        self.inner
            .into_inner()
            .unwrap_or_else(|err| err.into_inner())
    }

    /// Borrows the cell for handlers.
    #[must_use]
    pub fn as_ref(&self) -> ConnRef<'_, S> {
        ConnRef(self)
    }
}

/// A locally scoped reference shared connection handle. This construction ensures that the handle
/// stays in the concurrency model managed by the connection.
///
/// The following code is illegal:
///
/// ```ignore
/// async fn handler(conn: ConnRef<'_, S>) {
///     tokio::spawn(async move {
///         // This will NOT compile, as `conn` is not `'static` and instead bound to the function
///         // lifetime. By binding `conn` we can make assumtions about who is able to mutate the
///         // connection.
///         conn.do_something();
///     })
/// }
/// ```
pub struct ConnRef<'a, S>(&'a ConnCell<S>);

impl<S> Clone for ConnRef<'_, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for ConnRef<'_, S> {}

impl<S> std::fmt::Debug for ConnRef<'_, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnRef")
    }
}

impl<'a, S> ConnRef<'a, S> {
    /// Runs a closure with exclusive access to the connection and state. The closure is deliberately
    /// synchronous such that nothing can interleave it while keeping performance.
    ///
    /// # Panics
    ///
    /// Panics if called from inside another `with` on the same connection. Nothing else can
    /// contend for the lock: every handler is polled by the connection's own task, and a
    /// [`ConnRef`] cannot leave it. This cannot happen under normal circumstances.
    pub fn with<R>(self, f: impl FnOnce(&mut Conn<S>) -> R) -> R {
        let mut conn = match self.0.inner.try_lock() {
            Ok(conn) => conn,
            Err(TryLockError::Poisoned(err)) => err.into_inner(),
            Err(TryLockError::WouldBlock) => {
                // The public-facing API does not allow this to happen.
                panic!("ConnRef::with was called from inside another ConnRef::with")
            }
        };
        f(&mut conn)
    }

    /// The span the connection runs in: the parent of every handler span, and what a handler
    /// propagates when it hands the trace to another service.
    #[must_use]
    pub fn span(self) -> Span {
        self.with(|conn| conn.span().clone())
    }

    /// The protocol version the connection is in.
    #[must_use]
    pub fn version(self) -> ProtocolVersion {
        self.with(|conn| conn.version())
    }

    /// The phase the connection is in.
    #[must_use]
    pub fn phase(self) -> Phase {
        self.with(|conn| conn.phase())
    }

    /// The wire options this connection encodes with.
    #[must_use]
    pub fn options(self) -> Options {
        self.with(|conn| conn.options())
    }

    /// Encodes a packet against the current version and queues it for the wire.
    ///
    /// Shorthand for a `with` that only sends. Anything that has to be indivisible with this
    /// belongs in one [`with`](ConnRef::with) instead.
    ///
    /// # Errors
    ///
    /// Returns a [`ConnectionError::Codec`](crate::connection::ConnectionError::Codec) if the packet
    /// does not exist in the connection's version, or if its own encoder fails.
    pub fn send<P: Packet>(self, packet: P) -> Result<()> {
        self.with(|conn| conn.send(packet))
    }

    /// Schedules the connection to end after the queue has been cleared.
    pub fn close(self) {
        self.with(Conn::close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::NoCipher;
    use crate::common::versions;
    use crate::connection::ConnectionError;
    use crate::wire::{Reader, WireResult, Writer};
    use anyhow::anyhow;

    /// A packet that exists only from 26.1 on, so sending it at an older version is a mistake the
    /// encoder can catch.
    struct Recent;

    impl Packet for Recent {
        const NAME: &'static str = "Recent";
        const PHASE: Phase = Phase::Configuration;
        const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V26_3, 0x0B)];

        fn decode(_r: &mut Reader, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            Ok(())
        }
    }

    fn cell() -> ConnCell<Vec<&'static str>> {
        ConnCell::new(
            Vec::new(),
            versions::V26_3,
            Phase::Handshake,
            Options::default(),
        )
    }

    /// Everything currently queued, described the way `Out`'s own `Debug` does.
    fn drained(conn: ConnRef<'_, Vec<&'static str>>) -> Vec<String> {
        conn.with(|c| {
            let mut spare = Vec::new();
            c.swap_out(&mut spare);
            spare.iter().map(|out| format!("{out:?}")).collect()
        })
    }

    #[test]
    fn a_handler_applies_what_it_writes_where_it_writes_it() {
        // The difference from the queue this replaces: state, phase and version are not queued at
        // all. The handler holds the connection, so they are already true when the closure ends.
        let cell = cell();
        let conn = cell.as_ref();

        conn.with(|c| {
            c.set_version(versions::V26_3);
            c.set_phase(Phase::Login);
            c.state.push("recorded");
        });

        assert_eq!(conn.version(), versions::V26_3);
        assert_eq!(conn.phase(), Phase::Login);
        conn.with(|c| assert_eq!(c.state, vec!["recorded"]));
    }

    #[test]
    fn only_the_wire_waits_for_the_loop() {
        // Sends and cipher switches are the two things a handler cannot apply itself, so they are
        // the only two the outbox carries -- in the order they were written.
        let cell = cell();
        let conn = cell.as_ref();

        conn.with(|c| {
            c.send(Recent)?;
            c.encrypt(Box::new(NoCipher));
            c.send(Recent)
        })
        .expect("queues");

        assert_eq!(
            drained(conn),
            vec!["Frame(Recent)", "Cipher", "Frame(Recent)"]
        );
    }

    #[test]
    fn a_packet_is_encoded_against_the_version_the_connection_is_in() {
        // The reason `StaleEncoding` no longer exists: encoding happens while the connection is
        // held, so the version cannot have moved on between choosing it and using it.
        let cell = ConnCell::new(
            Vec::<&'static str>::new(),
            versions::V1_20_5,
            Phase::Handshake,
            Options::default(),
        );
        let conn = cell.as_ref();

        let error = conn
            .send(Recent)
            .expect_err("the packet does not exist that far back");
        assert!(matches!(error, ConnectionError::Codec(_)), "{error}");
        assert!(drained(conn).is_empty(), "nothing may be queued");

        conn.with(|c| c.set_version(versions::V26_3));
        conn.send(Recent).expect("it exists here");
        assert_eq!(drained(conn), vec!["Frame(Recent)"]);
    }

    #[test]
    fn one_closure_is_one_indivisible_change() {
        // A keep-alive handler cannot land a packet between the disconnect message and the
        // close, because it cannot run until this closure returns.
        let cell = cell();
        let conn = cell.as_ref();

        conn.with(|c| {
            c.send(Recent)?;
            c.set_phase(Phase::Configuration);
            c.close();
            Ok::<_, ConnectionError>(())
        })
        .expect("queues");

        assert_eq!(drained(conn), vec!["Frame(Recent)"]);
        assert_eq!(conn.phase(), Phase::Configuration);
        assert!(conn.with(|c| c.closing()));
    }

    #[test]
    fn the_gate_is_a_flag_the_loop_reads_not_a_lock() {
        // It starts open: a server reads the handshake before any handler has run.
        let cell = cell();
        let conn = cell.as_ref();

        assert!(!conn.with(|c| c.gated()));
        conn.with(Conn::gate);
        assert!(conn.with(|c| c.gated()));
        conn.with(Conn::release);
        assert!(!conn.with(|c| c.gated()));
    }

    #[test]
    fn the_outbox_keeps_its_allocation_across_rounds() {
        // The loop hands its emptied `Vec` over and takes the full one back, so the two buffers
        // cycle between them instead of one being allocated per round.
        let cell = cell();
        let conn = cell.as_ref();
        let mut spare = Vec::new();

        conn.send(Recent).expect("queues");
        conn.with(|c| c.swap_out(&mut spare));
        assert_eq!(spare.len(), 1, "the loop got what was queued");
        spare.clear();

        // The buffer the loop just emptied is what the connection queues into next.
        conn.send(Recent).expect("queues");
        conn.with(|c| c.swap_out(&mut spare));
        assert_eq!(spare.len(), 1);
        assert!(
            conn.with(|c| c.out.capacity()) > 0,
            "the emptied buffer went back to the connection rather than being dropped",
        );
    }

    #[test]
    #[should_panic(expected = "inside another ConnRef::with")]
    fn holding_the_connection_twice_is_a_bug_that_says_so() {
        // Nothing else can contend for the lock, so a failure to take it can only be this. It
        // panics rather than deadlocking, which is the reason for `try_lock`.
        let cell = cell();
        let conn = cell.as_ref();
        conn.with(|_| conn.with(|_| ()));
    }

    #[test]
    fn the_limits_are_the_handlers_to_move() {
        // A handler that says goodbye before the deadline reads it and gives itself room.
        let cell = cell();
        let conn = cell.as_ref();

        assert_eq!(conn.with(|c| c.deadline()), None);
        let at = Instant::now() + Duration::from_secs(30);
        conn.with(|c| c.set_deadline(Some(at)));
        assert_eq!(conn.with(|c| c.deadline()), Some(at));

        conn.with(|c| c.expire_in(Duration::from_secs(5)));
        let moved = conn.with(|c| c.deadline()).expect("a deadline");
        assert!(moved < at, "five seconds from now, not thirty");

        conn.with(|c| c.set_deadline(None));
        assert_eq!(conn.with(|c| c.deadline()), None, "and it can be removed");
    }

    #[test]
    fn detaching_leaves_the_cancellation_behind_rather_than_ignoring_it() {
        // A cancelled token stays cancelled, so a handler answering one takes a fresh token
        // instead; the loop reads back what it took.
        let cell = cell();
        let conn = cell.as_ref();

        let shutdown = CancellationToken::new();
        conn.with(|c| c.set_shutdown(shutdown.clone()));
        shutdown.cancel();
        assert!(conn.with(|c| c.shutdown().is_cancelled()));

        conn.with(Conn::detach);
        assert!(!conn.with(|c| c.shutdown().is_cancelled()));
        assert!(shutdown.is_cancelled(), "the old one is untouched");
    }

    #[test]
    fn the_first_failure_is_the_one_the_connection_ends_for() {
        // Everything after the first is its wake: a write to a socket nobody is reading, another
        // handler noticing the same hangup.
        let cell = cell();
        let conn = cell.as_ref();
        assert!(!conn.with(|c| c.closing()));

        conn.with(|c| c.fail(DispatchError::peer("refused", anyhow!("not today"))));
        conn.with(|c| c.fail(DispatchError::internal("broke", anyhow!("and then this"))));
        assert!(conn.with(|c| c.closing()));
        assert_eq!(
            conn.with(|c| c.take_error()).map(|error| error.reason()),
            Some("refused"),
        );
    }

    #[test]
    fn closing_ends_the_connection_without_a_reason_to_report() {
        let cell = cell();
        let conn = cell.as_ref();

        conn.with(Conn::close);
        assert!(conn.with(|c| c.closing()));
        assert!(conn.with(|c| c.take_error()).is_none());
    }

    #[test]
    fn the_span_a_handler_hands_on_is_the_connection_and_not_itself() {
        // What a transferred player's trace hangs off. A handler that propagates its own span makes
        // the backend a child of whichever packet happened to be in flight; the connection is what
        // the work actually belongs to, so that is what the cell remembers.
        tracing::subscriber::with_default(tracing_subscriber::registry(), || {
            let connection = tracing::info_span!("connection");
            let cell = connection.in_scope(cell);

            let handler = tracing::info_span!("on_login_login_acknowledged");
            handler.in_scope(|| {
                assert_ne!(
                    handler.id(),
                    connection.id(),
                    "two spans, or nothing to tell"
                );
                assert_eq!(Span::current().id(), handler.id());
                assert_eq!(cell.as_ref().span().id(), connection.id());
            });
        });
    }

    #[test]
    fn a_connection_hands_its_state_back_when_nothing_can_hold_it() {
        let cell = cell();
        cell.as_ref().with(|c| c.state.push("recorded"));
        assert_eq!(cell.into_inner().state, vec!["recorded"]);
    }
}
