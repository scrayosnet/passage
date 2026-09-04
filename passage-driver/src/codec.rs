//! The frame codec: length-prefixed frames, optional in-place encryption.
//!
//! The codec is deliberately *phase- and version-agnostic*. It turns a byte stream into
//! [`Frame`]s -- a packet ID plus an undecoded payload -- and back. Nothing here knows what a
//! packet is, which is what lets the same codec serve a client, a server, and a test harness.

use crate::error::{Error, InternalError, ProtocolError, Result};
use crate::packet::Packet;
use crate::version::ProtocolVersion;
use crate::wire::{Limits, Reader, Writer};
use bytes::{Bytes, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

/// A stream cipher applied to the raw byte stream.
///
/// The Minecraft protocol uses AES-128-CFB8, which is a byte-wise stream cipher with *separate*
/// encryption and decryption state. Keeping it behind a trait has two payoffs: the codec stays
/// dependency-free and testable, and the two halves can later be moved into separate tasks for a
/// full-duplex driver without touching the framing logic.
pub trait Cipher: Send + 'static {
    /// Encrypts `buf` in place, advancing the encryption state.
    fn encrypt(&mut self, buf: &mut [u8]);

    /// Decrypts `buf` in place, advancing the decryption state.
    fn decrypt(&mut self, buf: &mut [u8]);
}

/// So that [`FrameCodec`] can hold either a concrete cipher (monomorphised, inlinable) or a
/// `Box<dyn Cipher>` chosen at runtime, without two code paths.
impl<C: Cipher + ?Sized> Cipher for Box<C> {
    fn encrypt(&mut self, buf: &mut [u8]) {
        (**self).encrypt(buf);
    }

    fn decrypt(&mut self, buf: &mut [u8]) {
        (**self).decrypt(buf);
    }
}

/// A decoded frame: the packet ID and its still-undecoded payload.
///
/// The payload is a [`Bytes`] view into the read buffer, so frames the router does not know cost
/// nothing beyond the split.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The packet ID as it appeared on the wire.
    pub id: i32,

    /// The payload, excluding the length prefix and the ID.
    pub payload: Bytes,
}

/// A pre-encoded outbound packet: the ID varint followed by the payload, without a length prefix.
///
/// [`Bytes`] rather than a boxed packet, so queueing it is a move and the encode happened once, in
/// the handler that knew what it was sending.
#[derive(Clone, Debug)]
pub struct Encoded {
    /// The packet name, for tracing and metrics.
    pub name: &'static str,

    /// The ID varint followed by the payload.
    pub bytes: Bytes,
}

impl Encoded {
    /// Encodes a packet for the given protocol version.
    ///
    /// This is the only place an outbound packet ID is resolved, so no call site can drift from the
    /// ID table in the packet's own declaration.
    pub fn of<P: Packet>(packet: &P, version: ProtocolVersion, limits: Limits) -> Result<Self> {
        // Sending a packet that does not exist in the peer's version is an internal error, not a
        // silent no-op: the alternative is a client waiting forever for something we never sent.
        let id = P::id(version).ok_or(InternalError::PacketNotInVersion {
            packet: P::NAME,
            version,
        })?;

        let mut buf = BytesMut::with_capacity(64);
        {
            let mut writer = Writer::new(&mut buf, P::NAME, limits);
            writer.var_int(id);
            packet.encode(&mut writer, version)?;
        }

        if buf.len() > limits.max_frame_len {
            return Err(InternalError::OversizedFrame {
                packet: P::NAME,
                length: buf.len(),
                limit: limits.max_frame_len,
            }
            .into());
        }

        Ok(Self {
            name: P::NAME,
            bytes: buf.freeze(),
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
    limits: Limits,
    cipher: Option<C>,
    /// How many bytes of the read buffer have already been decrypted.
    decrypted: usize,
}

impl<C: Cipher> FrameCodec<C> {
    /// Creates a codec with the given limits and no encryption.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
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
    type Error = Error;

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
        let mut reader = Reader::new(src, self.limits);
        let length = match reader.var_int() {
            Ok(length) => length,
            Err(err) if err.is_incomplete() => return Ok(None),
            Err(err) => return Err(err),
        };
        if length < 0 {
            return Err(ProtocolError::NegativeLength {
                field: "frame_length",
                value: length,
            }
            .into());
        }
        let length = length as usize;
        if length > self.limits.max_frame_len {
            return Err(ProtocolError::FrameTooLarge {
                limit: self.limits.max_frame_len,
                length,
            }
            .into());
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

        let mut reader = Reader::new(&frame, self.limits);
        let id = reader.var_int()?;
        let id_len = reader.position();
        let payload = frame.split_off(id_len).freeze();
        Ok(Some(Frame { id, payload }))
    }
}

impl<C: Cipher> Encoder<Encoded> for FrameCodec<C> {
    type Error = Error;

    fn encode(&mut self, item: Encoded, dst: &mut BytesMut) -> Result<()> {
        let plain_until = dst.len();
        {
            let mut writer = Writer::new(dst, item.name, self.limits);
            // `Encoded::bytes` already contains the ID varint, so its length is the frame length.
            // `length` refuses anything that would not fit, rather than wrapping the cast.
            writer.length(item.bytes.len())?;
            writer.raw(&item.bytes);
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

    /// A trivially reversible stand-in for AES-CFB8 that is *stateful*, so a mistake in the
    /// encrypt-once accounting shows up as garbage rather than silently passing.
    struct RotatingCipher {
        encrypt_offset: u8,
        decrypt_offset: u8,
    }

    impl RotatingCipher {
        fn new() -> Self {
            Self {
                encrypt_offset: 0,
                decrypt_offset: 0,
            }
        }
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

    fn encoded(id: i32, payload: &[u8]) -> Encoded {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf, "Test", Limits::default());
        writer.var_int(id);
        writer.raw(payload);
        Encoded {
            name: "Test",
            bytes: buf.freeze(),
        }
    }

    #[test]
    fn frames_roundtrip() {
        let mut codec = FrameCodec::<RotatingCipher>::new(Limits::default());
        let mut buf = BytesMut::new();
        codec
            .encode(encoded(0x42, b"hello"), &mut buf)
            .expect("encodes");

        let frame = codec.decode(&mut buf).expect("decodes").expect("complete");
        assert_eq!(frame.id, 0x42);
        assert_eq!(&frame.payload[..], b"hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn partial_frames_are_not_an_error() {
        let mut codec = FrameCodec::<RotatingCipher>::new(Limits::default());
        let mut buf = BytesMut::new();
        codec
            .encode(encoded(0x01, b"0123456789"), &mut buf)
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
    fn oversized_frames_are_rejected_without_buffering() {
        let mut codec = FrameCodec::<RotatingCipher>::new(Limits {
            max_frame_len: 64,
            ..Limits::default()
        });
        let mut buf = BytesMut::new();
        // Announce a 1 MiB frame in three bytes and send nothing else.
        Writer::new(&mut buf, "Test", Limits::default()).var_int(1024 * 1024);

        let err = codec.decode(&mut buf).expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn an_oversized_frame_is_refused_on_the_way_out_too() {
        // The mirror of the test above: our own bug must not become the peer's parsing problem.
        let mut codec = FrameCodec::<RotatingCipher>::new(Limits {
            max_frame_len: 16,
            ..Limits::default()
        });
        let err = codec
            .encode(encoded(0x00, &[0u8; 64]), &mut BytesMut::new())
            .expect_err("must refuse");
        assert!(matches!(
            err,
            Error::Internal(InternalError::OversizedFrame { .. })
        ));
    }

    #[test]
    fn encryption_applies_from_the_switchover_point_only() {
        // A concrete cipher: no `Box`, and `encrypt`/`decrypt` are static calls the compiler
        // can inline into the framing loop.
        let mut server = FrameCodec::<RotatingCipher>::new(Limits::default());
        let mut client = FrameCodec::<RotatingCipher>::new(Limits::default());
        let mut wire = BytesMut::new();

        // Plaintext frame, then enable encryption on both ends, then two encrypted frames.
        server
            .encode(encoded(0x00, b"plain"), &mut wire)
            .expect("encodes");
        server.set_cipher(RotatingCipher::new());
        server
            .encode(encoded(0x01, b"secret"), &mut wire)
            .expect("encodes");
        server
            .encode(encoded(0x02, b"more"), &mut wire)
            .expect("encodes");

        // The receiver must decode the plaintext frame *before* switching, mirroring the sender.
        let frame = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&frame.payload[..], b"plain");
        client.set_cipher(RotatingCipher::new());

        let frame = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&frame.payload[..], b"secret");
        let frame = client
            .decode(&mut wire)
            .expect("decodes")
            .expect("complete");
        assert_eq!(&frame.payload[..], b"more");
    }
}
