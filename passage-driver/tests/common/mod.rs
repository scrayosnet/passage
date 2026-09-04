//! A raw protocol client, shared by the test binaries.
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
use tokio::io::DuplexStream;
use tokio_util::codec::Framed;

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
