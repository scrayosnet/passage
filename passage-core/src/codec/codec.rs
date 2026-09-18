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
