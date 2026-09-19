use crate::codec::cipher::Cipher;
use crate::codec::error::{CodecError, Result};
use crate::packet::Packet;
use crate::version::ProtocolVersion;
use crate::wire::{Options, Reader, WireError, Writer};
use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// The packet name used for packets with an unknown name. The name is only used for metrics and traces.
pub const UNKNOWN_PACKET_NAME: &str = "[unknown]";

/// An encoded packet for the current protocol version. It contains the packet ID and its payload
/// without the length prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The packet name (if known), for tracing and metrics. This is generally only known for packets
    /// that should be written *to* the wire, not for packets that are read.
    pub name: &'static str,

    /// The packet ID, decoded from [`payload`](Frame::payload).
    pub id: i32,

    /// The packet ID followed by the packet payload, excluding the length prefix.
    pub payload: Bytes,
}

impl Frame {
    /// Encodes a packet for the given protocol version.
    ///
    /// This is the only place an outbound packet ID is resolved, so no call site can drift from the
    /// ID table in the packet's own declaration and no packet has to write its own ID.
    ///
    /// # Errors
    ///
    /// Returns a [`CodecError::PacketNotInVersion`] if the packet does not exist in `version`, a
    /// [`CodecError::OversizedFrame`] if the encoded packet does not fit a frame, and any error the
    /// packet's own encoder raises.
    pub fn of<P: Packet>(packet: &P, version: ProtocolVersion, options: Options) -> Result<Self> {
        // Sending a packet that does not exist in the peer's version is an internal error, not a
        // silent no-op: the alternative is a client waiting forever for something we never sent.
        let id = P::id(version).ok_or(CodecError::PacketNotInVersion {
            packet: P::NAME,
            version,
        })?;

        // TODO try to reuse the same buffer for multiple packets (use const bytesBut with reserve/split)?
        let mut buf = BytesMut::with_capacity(64);
        {
            let mut writer = Writer::new(&mut buf).with_options(options);
            // The ID leads the payload, exactly as the decoder will find it.
            writer.var_int(id);
            packet.encode(&mut writer, version)?;
        }

        if buf.len() > options.max_frame_len {
            return Err(CodecError::OversizedFrame {
                packet: P::NAME,
                length: buf.len(),
                limit: options.max_frame_len,
            });
        }

        Ok(Self {
            name: P::NAME,
            id,
            payload: buf.freeze(),
        })
    }
}

/// Frames the Minecraft packet format: `VarInt` length, `VarInt` ID, payload.
///
/// Generic over the cipher so a caller with a known cipher type gets static dispatch and an
/// inlinable `encrypt`/`decrypt`. `Box<dyn Cipher>` is the default because that is what travels
/// through the connection's operation queue, where the concrete type is not known until the encryption
/// handshake picks it.
pub struct FrameCodec<C: Cipher = Box<dyn Cipher>> {
    options: Options,
    cipher: Option<C>,
    /// How many bytes of the read buffer have already been decrypted.
    decrypted: usize,
}

impl<C: Cipher> FrameCodec<C> {
    /// Creates a codec with the given options and no encryption.
    #[must_use]
    pub fn new(options: Options) -> Self {
        Self {
            options,
            cipher: None,
            decrypted: 0,
        }
    }

    /// Enables encryption from this point in the stream on.
    ///
    /// Correctness depends entirely on *when* this is called: every byte written before must be
    /// plaintext and every byte after must be ciphertext. The connection therefore routes this through
    /// the same ordered operation queue as packet sends instead of exposing it to handlers.
    pub fn set_cipher(&mut self, cipher: C) {
        self.cipher = Some(cipher);
    }

    /// Whether the stream is encrypted.
    #[must_use]
    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }
}

impl<C: Cipher> Decoder for FrameCodec<C> {
    type Item = Frame;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>> {
        // Decrypt everything that arrived since the last call, in place. CFB8 is byte-wise, so any
        // split is safe as long as each byte is decrypted exactly once -- which `decrypted` tracks.
        if let Some(cipher) = self.cipher.as_mut()
            && src.len() > self.decrypted
        {
            cipher.decrypt(&mut src[self.decrypted..]);
            self.decrypted = src.len();
        }

        // Read the length prefix. Too few bytes is not an error, it is backpressure.
        let mut reader = Reader::new(src).with_options(self.options);
        let length = match reader.var_int("packet_length") {
            Ok(length) => length,
            Err(WireError::Eof { .. }) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        if length < 0 {
            return Err(WireError::NegativeLength {
                field: "frame_length",
                value: length,
            }
            .into());
        }
        let length = length as usize;
        if length > self.options.max_frame_len {
            return Err(CodecError::OversizedFrame {
                packet: UNKNOWN_PACKET_NAME,
                limit: self.options.max_frame_len,
                length,
            });
        }

        // Wait for the full frame. Reserving up front avoids repeated growth for large frames.
        let header = reader.position();
        let total = header + length;
        if src.len() < total {
            src.reserve(total - src.len());
            return Ok(None);
        }

        // Split the frame off. The remaining buffer keeps its decrypted prefix accounting.
        let mut frame = src.split_to(total);
        self.decrypted = self.decrypted.saturating_sub(total);
        let _prefix = frame.split_to(header);

        // The ID is read for routing but left in the payload, so that what we decode has the same
        // shape as what `Frame::of` builds.
        let mut reader = Reader::new(&frame).with_options(self.options);
        let id = reader.var_int("packet_id")?;
        let payload = frame.freeze();
        Ok(Some(Frame {
            name: UNKNOWN_PACKET_NAME,
            id,
            payload,
        }))
    }
}

impl<C: Cipher> Encoder<Frame> for FrameCodec<C> {
    type Error = CodecError;

    fn encode(&mut self, item: Frame, dst: &mut BytesMut) -> Result<()> {
        let plain_until = dst.len();
        {
            let mut writer = Writer::new(dst).with_options(self.options);
            writer.length("packet_length", item.payload.len())?;
            writer.raw(&item.payload);
        }

        if let Some(cipher) = self.cipher.as_mut() {
            cipher.encrypt(&mut dst[plain_until..]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase::Phase;
    use crate::version::versions;
    use crate::wire::WireResult;

    /// A trivially reversible stand-in for AES-CFB8 that is *stateful*, so a mistake in the
    /// encrypt-once accounting shows up as garbage rather than silently passing.
    #[derive(Default)]
    struct RotatingCipher {
        encrypt_offset: u8,
        decrypt_offset: u8,
    }

    impl Cipher for RotatingCipher {
        fn encrypt(&mut self, buf: &mut [u8]) {
            for byte in buf {
                *byte = byte.wrapping_add(self.encrypt_offset);
                self.encrypt_offset = self.encrypt_offset.wrapping_add(1);
            }
        }

        fn decrypt(&mut self, buf: &mut [u8]) {
            for byte in buf {
                *byte = byte.wrapping_sub(self.decrypt_offset);
                self.decrypt_offset = self.decrypt_offset.wrapping_add(1);
            }
        }
    }

    /// A packet whose ID moved, so that resolving it is a version decision rather than a constant.
    struct Greeting {
        text: String,
    }

    impl Packet for Greeting {
        const NAME: &'static str = "Greeting";
        const PHASE: Phase = Phase::Login;
        const IDS: &'static [(ProtocolVersion, i32)] =
            &[(versions::V26_2, 0x42), (versions::V1_20_5, 0x07)];

        fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
            Ok(Self {
                text: r.string("text", 64)?,
            })
        }

        fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
            w.string("text", &self.text)
        }
    }

    fn codec() -> FrameCodec<RotatingCipher> {
        FrameCodec::new(Options::default())
    }

    /// A frame with `id` and a payload of raw bytes, the way a peer would put it on the wire.
    fn frame(id: i32, payload: &[u8]) -> Frame {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        writer.var_int(id);
        writer.raw(payload);
        Frame {
            name: "Test",
            id,
            payload: buf.freeze(),
        }
    }

    #[test]
    fn frames_roundtrip_without_cipher() {
        let mut codec = codec();
        let mut buf = BytesMut::new();
        codec
            .encode(frame(0x42, b"hello"), &mut buf)
            .expect("encodes");

        let decoded = codec.decode(&mut buf).expect("decodes").expect("complete");
        assert_eq!(decoded.id, 0x42);
        assert_eq!(&decoded.payload[..], b"\x42hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn the_id_is_on_the_wire_and_stays_in_the_payload() {
        // Both halves of one rule: `Frame::of` writes the ID so that no packet has to, and `decode`
        // leaves it in place so that what arrives has the same shape as what was built. A codec that
        // wrote no ID would put the payload's first byte where the peer looks for one.
        let encoded = Frame::of(
            &Greeting {
                text: "hi".to_owned(),
            },
            versions::V26_2,
            Options::default(),
        )
        .expect("the packet exists in 26.2");
        assert_eq!(encoded.id, 0x42);
        assert_eq!(&encoded.payload[..], b"\x42\x02hi");

        let mut buf = BytesMut::new();
        codec().encode(encoded, &mut buf).expect("encodes");
        assert_eq!(&buf[..], b"\x04\x42\x02hi", "length, id, then the payload");

        let decoded = codec()
            .decode(&mut buf)
            .expect("decodes")
            .expect("complete");
        assert_eq!(decoded.id, 0x42);
        // The router skips the ID again before handing the rest to `Packet::decode`.
        let mut reader = Reader::new(&decoded.payload);
        assert_eq!(reader.var_int("packet_id").expect("reads"), 0x42);
        let greeting = Greeting::decode(&mut reader, versions::V26_2).expect("decodes");
        assert_eq!(greeting.text, "hi");
        reader.finish(Greeting::NAME).expect("nothing is left");
    }

    #[test]
    fn an_id_is_resolved_against_the_version_it_is_sent_at() {
        let older = Frame::of(
            &Greeting {
                text: String::new(),
            },
            versions::V1_20_5,
            Options::default(),
        )
        .expect("the packet exists in 1.20.5");
        assert_eq!(older.id, 0x07);

        // Below the oldest entry the packet does not exist. Sending it anyway would leave a peer
        // waiting forever for something we never sent, so it is an error rather than a no-op.
        let err = Frame::of(
            &Greeting {
                text: String::new(),
            },
            ProtocolVersion::new(765),
            Options::default(),
        )
        .expect_err("must refuse");
        assert!(
            matches!(
                err,
                CodecError::PacketNotInVersion {
                    packet: "Greeting",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn partial_frames_are_not_an_error() {
        let mut codec = codec();
        let mut buf = BytesMut::new();
        codec
            .encode(frame(0x01, b"0123456789"), &mut buf)
            .expect("encodes");

        // Feed the frame byte by byte; only the last byte may produce a frame.
        let full = buf.split();
        let mut partial = BytesMut::new();
        for (index, byte) in full.iter().enumerate() {
            partial.extend_from_slice(&[*byte]);
            let decoded = codec.decode(&mut partial).expect("no error");
            assert_eq!(
                decoded.is_some(),
                index + 1 == full.len(),
                "at byte {index}"
            );
        }
    }

    #[test]
    fn two_frames_in_one_read_are_both_decoded() {
        // A peer is free to write a whole conversation in one syscall, and the codec is called once
        // per frame rather than once per read.
        let mut codec = codec();
        let mut buf = BytesMut::new();
        codec
            .encode(frame(0x01, b"first"), &mut buf)
            .expect("encodes");
        codec
            .encode(frame(0x02, b"second"), &mut buf)
            .expect("encodes");

        let first = codec.decode(&mut buf).expect("decodes").expect("complete");
        let second = codec.decode(&mut buf).expect("decodes").expect("complete");
        assert_eq!((first.id, second.id), (0x01, 0x02));
        assert!(codec.decode(&mut buf).expect("no error").is_none());
    }

    #[test]
    fn oversized_frames_are_rejected_without_buffering() {
        let mut codec = FrameCodec::<RotatingCipher>::new(Options {
            max_frame_len: 64,
            ..Options::default()
        });
        let mut buf = BytesMut::new();
        // Announce a 1 MiB frame in three bytes and send nothing else.
        Writer::new(&mut buf).var_int(1024 * 1024);

        let err = codec.decode(&mut buf).expect_err("must reject");
        assert!(
            matches!(err, CodecError::OversizedFrame { limit: 64, .. }),
            "{err}"
        );
    }

    #[test]
    fn a_negative_frame_length_is_refused_rather_than_cast() {
        let mut codec = codec();
        let mut buf = BytesMut::new();
        Writer::new(&mut buf).var_int(-1);

        let err = codec.decode(&mut buf).expect_err("must reject");
        assert!(
            matches!(
                err,
                CodecError::Wire(WireError::NegativeLength { value: -1, .. })
            ),
            "{err}"
        );
    }

    #[test]
    fn an_oversized_frame_is_refused_on_the_way_out_too() {
        // The mirror of the test above: our own bug must not become the peer's parsing problem.
        let err = Frame::of(
            &Greeting {
                text: "x".repeat(64),
            },
            versions::V26_2,
            Options {
                max_frame_len: 16,
                ..Options::default()
            },
        )
        .expect_err("must refuse");
        // The field-level limit is the frame, so it is the writer that says no first.
        assert!(
            matches!(
                err,
                CodecError::Wire(WireError::LengthLimit { limit: 16, .. })
            ),
            "{err}"
        );
    }

    #[test]
    fn encryption_applies_from_the_switchover_point_only() {
        // A concrete cipher: no `Box`, and `encrypt`/`decrypt` are static calls the compiler can
        // inline into the framing loop.
        let mut server = codec();
        let mut client = codec();
        let mut wire = BytesMut::new();

        // Plaintext frame, then enable encryption on both ends, then two encrypted frames.
        server
            .encode(frame(0x00, b"plain"), &mut wire)
            .expect("encodes");
        assert!(!server.is_encrypted());
        server.set_cipher(RotatingCipher::default());
        assert!(server.is_encrypted());
        server
            .encode(frame(0x01, b"secret"), &mut wire)
            .expect("encodes");
        server
            .encode(frame(0x02, b"more"), &mut wire)
            .expect("encodes");

        // The receiver must decode the plaintext frame *before* switching, mirroring the sender.
        let decoded = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&decoded.payload[1..], b"plain");
        client.set_cipher(RotatingCipher::default());

        let decoded = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&decoded.payload[1..], b"secret");
        let decoded = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&decoded.payload[1..], b"more");
    }

    #[test]
    fn a_ciphertext_byte_is_decrypted_exactly_once() {
        // The accounting `decrypted` exists for: a frame that arrives in pieces must not have its
        // buffered prefix run through the cipher a second time.
        let mut server = codec();
        let mut client = codec();
        server.set_cipher(RotatingCipher::default());
        client.set_cipher(RotatingCipher::default());

        let mut wire = BytesMut::new();
        server
            .encode(frame(0x01, b"0123456789"), &mut wire)
            .expect("encodes");
        server
            .encode(frame(0x02, b"tail"), &mut wire)
            .expect("encodes");

        let full = wire.split();
        let mut partial = BytesMut::new();
        let mut decoded = Vec::new();
        for byte in full.iter() {
            partial.extend_from_slice(&[*byte]);
            while let Some(frame) = client.decode(&mut partial).expect("no error") {
                decoded.push(frame);
            }
        }
        assert_eq!(decoded.len(), 2);
        assert_eq!(&decoded[0].payload[1..], b"0123456789");
        assert_eq!(&decoded[1].payload[1..], b"tail");
    }

    #[test]
    fn a_boxed_cipher_is_the_one_the_connection_uses() {
        // The default type parameter: the concrete cipher is not known until the encryption
        // handshake picks it, so it travels as a `Box` through the operation queue.
        let mut codec = FrameCodec::default_boxed();
        codec.set_cipher(Box::new(RotatingCipher::default()));
        let mut wire = BytesMut::new();
        codec
            .encode(frame(0x01, b"secret"), &mut wire)
            .expect("encodes");

        let mut peer = FrameCodec::default_boxed();
        peer.set_cipher(Box::new(RotatingCipher::default()));
        let decoded = peer.decode(&mut wire).expect("decodes").expect("complete");
        assert_eq!(&decoded.payload[1..], b"secret");
    }

    impl FrameCodec<Box<dyn Cipher>> {
        /// The codec a connection builds, spelled once for the test above.
        fn default_boxed() -> Self {
            Self::new(Options::default())
        }
    }
}
