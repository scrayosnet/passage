//! A client with no connection loop, for tests that are about bytes rather than about routing.

use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
use passage_core::codec::{Frame, FrameCodec};
use passage_core::wire::{Options, Reader, Writer};
use passage_core::{Packet, ProtocolVersion};
use tokio::io::{AsyncWriteExt, DuplexStream};
use tokio_util::codec::Framed;

/// A peer that frames packets and nothing else.
///
/// The [`Scenario`](super::Scenario) is the readable way to write a test; this is for the cases a
/// router cannot express, because what is being sent is not a packet at all: an ID nobody
/// registered, a length prefix that lies, a frame that stops halfway.
pub struct RawClient {
    /// Reached into directly by tests that need to look at a frame, or at the socket under it.
    pub framed: Framed<DuplexStream, FrameCodec>,
    /// The version this encodes and decodes with; a test pins it as the handshake would.
    pub version: ProtocolVersion,
    /// The options both directions are read and written with.
    pub options: Options,
}

impl RawClient {
    /// Wraps one half of a socket pair.
    pub fn new(io: DuplexStream) -> Self {
        let options = Options::default();
        Self {
            framed: Framed::new(io, FrameCodec::new(options)),
            version: ProtocolVersion::UNKNOWN,
            options,
        }
    }

    /// Pins the version, the way the handshake does for both sides at once.
    pub fn at(mut self, version: ProtocolVersion) -> Self {
        self.version = version;
        self
    }

    /// Sends a packet, encoded for the version this client is pinned at.
    pub async fn send<P: Packet>(&mut self, packet: &P) {
        let frame = Frame::of(packet, self.version, self.options)
            .expect("the packet exists in this version");
        self.framed.send(frame).await.expect("writes");
    }

    /// Sends an ID and a payload that no packet has to account for.
    pub async fn send_raw(&mut self, id: i32, payload: &[u8]) {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        writer.var_int(id);
        writer.raw(payload);
        self.framed
            .send(Frame {
                name: "Raw",
                id,
                payload: buf.freeze(),
            })
            .await
            .expect("writes");
    }

    /// Writes bytes straight to the socket, under the framing rather than through it.
    pub async fn send_bytes(&mut self, bytes: &[u8]) {
        self.framed
            .get_mut()
            .write_all(bytes)
            .await
            .expect("writes");
    }

    /// Reads the next packet, asserting it is the one expected.
    pub async fn expect<P: Packet>(&mut self) -> P {
        let frame = self.next_frame().await;
        assert_eq!(
            Some(frame.id),
            P::id(self.version),
            "expected {} (id {:?}), got id {:#04x}",
            P::NAME,
            P::id(self.version),
            frame.id,
        );
        let mut reader = Reader::new(&frame.payload).with_options(self.options);
        reader
            .var_int("packet_id")
            .expect("the ID leads the payload");
        let packet = P::decode(&mut reader, self.version).expect("packet decodes");
        reader
            .finish(P::NAME)
            .expect("the whole payload is accounted for");
        packet
    }

    /// Reads the next frame, whatever it turns out to be.
    pub async fn next_frame(&mut self) -> Frame {
        self.framed
            .next()
            .await
            .expect("the connection stays open")
            .expect("the frame decodes")
    }

    /// Asserts that the peer closed the connection.
    pub async fn expect_eof(&mut self) {
        assert!(
            self.framed.next().await.is_none(),
            "expected the peer to close the connection",
        );
    }
}
