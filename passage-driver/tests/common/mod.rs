//! A raw protocol client and a log recorder, shared by the test binaries.
//!
//! Not every test uses every method, and each test binary compiles this module separately, so
//! unused ones are expected here.
#![allow(dead_code)]

use futures::{SinkExt, StreamExt};
use passage_driver::codec::{Encoded, FrameCodec};
use passage_driver::demo::packets::{Intent, Intention};
use passage_driver::packet::Packet;
use passage_driver::version::ProtocolVersion;
use passage_driver::wire::{Limits, Reader};
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use tokio::io::DuplexStream;
use tokio_util::codec::Framed;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

/// Everything the driver logged, so a test can assert on what an operator would see.
///
/// This is how the server's own reporting is tested now that there is no hook to observe: the thing
/// under test *is* the log line, so the test reads the log line.
#[derive(Clone, Default)]
pub struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    /// Every event recorded so far, as `(level, "message field=value ...")`.
    pub fn events(&self) -> Vec<(Level, String)> {
        self.0.lock().expect("not poisoned").clone()
    }

    /// The one event whose text contains `needle`, and its level.
    ///
    /// Panics unless exactly one matches, so a test cannot quietly assert on the wrong line.
    pub fn find(&self, needle: &str) -> (Level, String) {
        let events = self.events();
        let mut matching = events.iter().filter(|(_, text)| text.contains(needle));
        let found = matching
            .next()
            .unwrap_or_else(|| panic!("no event mentioning {needle:?} in {events:#?}"))
            .clone();
        assert!(
            matching.next().is_none(),
            "more than one event mentioning {needle:?} in {events:#?}",
        );
        found
    }
}

/// Records everything logged for as long as the guard is held.
///
/// Thread-local rather than global, which is what lets every test have its own: `#[tokio::test]`
/// runs on a current-thread runtime, so the connection tasks the server spawns land on this same
/// thread and see it.
pub fn record_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(Recorder(logs.clone()));
    let guard = tracing::subscriber::set_default(subscriber);
    (logs, guard)
}

struct Recorder(Logs);

impl<S: Subscriber + for<'a> LookupSpan<'a>> tracing_subscriber::Layer<S> for Recorder {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut text = Flatten(String::new());
        event.record(&mut text);
        self.0
            .0
            .lock()
            .expect("not poisoned")
            .push((*event.metadata().level(), text.0));
    }
}

/// Renders an event as one string, so assertions can be about what it says rather than about how
/// tracing structures it.
struct Flatten(String);

impl Visit for Flatten {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

/// A minimal protocol client: the mirror image of the server, without the connection loop.
pub struct TestClient {
    /// Reached into directly by tests that need to look at a raw frame.
    pub framed: Framed<DuplexStream, FrameCodec>,
    /// The version the client encodes and decodes with; tests pin it as the handshake does.
    pub version: ProtocolVersion,
    pub limits: Limits,
}

impl TestClient {
    pub fn new(io: DuplexStream) -> Self {
        let limits = Limits::default();
        Self {
            framed: Framed::new(io, FrameCodec::new(limits)),
            version: ProtocolVersion::UNKNOWN,
            limits,
        }
    }

    pub async fn send<P: Packet>(&mut self, packet: &P) {
        let encoded =
            Encoded::of(packet, self.version, self.limits).expect("packet exists in this version");
        self.framed.send(encoded).await.expect("writes");
    }

    /// Reads the next packet, asserting it is of the expected type.
    pub async fn expect<P: Packet>(&mut self) -> P {
        let frame = self
            .framed
            .next()
            .await
            .expect("connection stays open")
            .expect("frame decodes");
        assert_eq!(
            Some(frame.id),
            P::id(self.version),
            "expected {} (id {:?}), got id {}",
            P::NAME,
            P::id(self.version),
            frame.id,
        );
        let mut reader = Reader::new(&frame.payload, self.limits);
        P::decode(&mut reader, self.version).expect("packet decodes")
    }

    /// Sends bytes that are not a valid packet at all.
    pub async fn send_raw(&mut self, bytes: &[u8]) {
        self.framed
            .send(Encoded {
                name: "Raw",
                bytes: bytes.to_vec().into(),
            })
            .await
            .expect("writes");
    }

    pub async fn expect_eof(&mut self) {
        assert!(
            self.framed.next().await.is_none(),
            "expected the server to close the connection",
        );
    }
}

/// The handshake packet, for a given version and intent.
pub fn intention(version: ProtocolVersion, intent: Intent) -> Intention {
    intention_to("mc.justchunks.net", version, intent)
}

/// The handshake packet, for a given hostname -- which is what the session records as its host.
pub fn intention_to(host: &str, version: ProtocolVersion, intent: Intent) -> Intention {
    Intention {
        protocol_version: version,
        server_address: host.to_owned(),
        server_port: 25565,
        intent,
    }
}
