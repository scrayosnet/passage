use crate::codec::{Cipher, Frame};
use crate::common::Phase;
use crate::common::ProtocolVersion;
use crate::connection::error::Result;
use crate::packet::packet::Packet;
use crate::wire::Options;
use std::sync::{Mutex, TryLockError};

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
    /// break rather than input to be handled later.
    gated: bool,

    /// Whether the connection should end once the queue has been cleared.
    closing: bool,
}

impl<S> Conn<S> {
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

    /// Disables reading new packet frames until [`release`](Conn::release) is called.
    pub fn gate(&mut self) {
        self.gated = true;
    }

    /// Enables reading new packet frames after [`gate`](Conn::gate) was called.
    pub fn release(&mut self) {
        self.gated = false;
    }

    /// Whether new packet frames will currently be read.
    #[must_use]
    pub fn gated(&self) -> bool {
        self.gated
    }

    /// Schedules the connection to end after the queue has been cleared.
    pub fn close(&mut self) {
        self.closing = true;
    }

    /// Whether the connection is scheduled to end.
    #[must_use]
    pub fn closing(&self) -> bool {
        self.closing
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
    pub(crate) fn new(state: S, version: ProtocolVersion, phase: Phase, options: Options) -> Self {
        Self {
            inner: Mutex::new(Conn {
                state,
                out: Vec::new(),
                version,
                phase,
                options,
                gated: false,
                closing: false,
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
    use crate::common::versions;
    use crate::connection::ConnectionError;
    use crate::wire::{Reader, WireResult, Writer};

    /// A packet that exists only from 26.1 on, so sending it at an older version is a mistake the
    /// encoder can catch.
    struct Recent;

    impl Packet for Recent {
        const NAME: &'static str = "Recent";
        const PHASE: Phase = Phase::Configuration;
        const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V26_1, 0x0B)];

        fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self)
        }

        fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            Ok(())
        }
    }

    fn cell() -> ConnCell<Vec<&'static str>> {
        ConnCell::new(
            Vec::new(),
            versions::V26_1,
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
            c.set_version(versions::V26_1);
            c.set_phase(Phase::Login);
            c.state.push("recorded");
        });

        assert_eq!(conn.version(), versions::V26_1);
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

        conn.with(|c| c.set_version(versions::V26_1));
        conn.send(Recent).expect("it exists here");
        assert_eq!(drained(conn), vec!["Frame(Recent)"]);
    }

    #[test]
    fn one_closure_is_one_indivisible_change() {
        // What `Batch` used to be for. A tick handler cannot land a keep-alive between the
        // disconnect message and the close, because it cannot run until this closure returns.
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
    fn a_connection_hands_its_state_back_when_nothing_can_hold_it() {
        let cell = cell();
        cell.as_ref().with(|c| c.state.push("recorded"));
        assert_eq!(cell.into_inner().state, vec!["recorded"]);
    }

    /// A cipher that does nothing, for tests that only care that the switch was queued.
    struct NoCipher;

    impl Cipher for NoCipher {
        fn encrypt(&mut self, _buf: &mut [u8]) {}
        fn decrypt(&mut self, _buf: &mut [u8]) {}
    }
}
