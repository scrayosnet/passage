use crate::io::reader::{ReadPacket, ReadPacketExt};
use crate::io::writer::{WritePacket, WritePacketExt};
use crate::{Error, VarInt};
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, BlockSizeUser, InvalidLength};
use bytes::{BufMut, Bytes};
use cfb8::cipher::KeyIvInit;
use std::io::{Cursor, Write};
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

/// A macro for pattern matching on packet frames based on their packet ID.
///
/// This macro provides a convenient way to decode and handle different packet types from a
/// [`PacketFrame`] without manually checking packet IDs. It automatically extracts the packet ID,
/// reads the appropriate packet type, and executes the corresponding handler.
///
/// # Syntax
///
/// ```ignore
/// match_packet!(frame,
///     pattern = PacketType => handler_expression,
///     pattern = PacketType => handler_expression,
///     // ... more packet arms
///     fallback_pattern => fallback_handler,
/// )
/// ```
///
/// # Parameters
///
/// * `frame` - A [`PacketFrame`] containing the packet data to match against
/// * `pattern` - A pattern to bind the decoded packet (e.g., `packet`, `_`, destructured pattern)
/// * `PacketType` - The packet type that implements [`ReadPacket`] and [`Packet`]
/// * `handler_expression` - The code to execute when the packet ID matches
/// * `fallback_pattern` - A pattern to bind the unmatched packet ID (typically `_` or a variable name)
/// * `fallback_handler` - The code to execute when no packet types match
///
/// # Examples
///
/// ```ignore
/// use passage_packets::io::codec::PacketFrame;
/// use passage_packets::handshake::serverbound::HandshakePacket;
/// use passage_packets::status::serverbound::{StatusRequestPacket, PingRequestPacket};
///
/// fn handle_packet(frame: PacketFrame) {
///     match_packet!(frame,
///         packet = HandshakePacket => {
///             println!("Received handshake: {:?}", packet);
///         },
///         packet = StatusRequestPacket => {
///             println!("Received status request");
///         },
///         packet = PingRequestPacket => {
///             println!("Received ping: {:?}", packet);
///         },
///         id => {
///             println!("Unknown packet ID: {}", id);
///         },
///     )
/// }
/// ```
#[macro_export]
macro_rules! match_packet {
    ($frame:expr, $($arms:tt)*) => {{
        let __match_packet_frame = $frame;
        let __match_packet_id = __match_packet_frame.id;
        let mut __match_packet_reader = std::io::Cursor::new(&__match_packet_frame.data);

        match_packet!(@dispatch
            __match_packet_id,
            __match_packet_reader;
            $($arms)*
        )
    }};

    (@dispatch $id:expr, $reader:ident;
        $packet_bind:pat = $packet_type:ty => $packet_handler:expr,
        $($rest:tt)*
    ) => {{
        if $id == <$packet_type as $crate::Packet>::ID {
            let $packet_bind = <$packet_type as $crate::io::reader::ReadPacket>::read_packet(&mut $reader);
            $packet_handler
        } else {
            match_packet!(@dispatch $id, $reader; $($rest)*)
        }
    }};

    (@dispatch $id:expr, $reader:ident;
        $else_bind:pat => $else_handler:expr $(,)?
    ) => {{
        let $else_bind = $id;
        $else_handler
    }};
}

/// The cipher used by the Minecraft protocol (AES-128-CFB8).
pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;

/// The cipher used by the Minecraft protocol (AES-128-CFB8).
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

/// Creates a cipher pair for the Minecraft protocol (AES-128-CFB8) using a shared secret.
pub fn ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), InvalidLength> {
    let encryptor = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decryptor = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;
    Ok((encryptor, decryptor))
}

/// A [`PacketFrame`] represents a packet that has been read from the network as a frame following the
/// [official packet format](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Packet_format).
///
/// The frame can then be parsed into a packet using the [`PacketFrame::into_packet`] method. This
/// mechanism allows the frame to remain oblivious to which packet it is until it is parsed.
pub struct PacketFrame {
    /// The length of the packet in bytes, including the packet ID field.
    pub length: usize,

    /// The ID of the packet. Compare against the [`Packet::ID`] constant to check if the packet is.
    pub id: VarInt,

    /// The packet data. This should be read by the [`ReadPacket::read_packet`].
    pub data: Bytes,
}

impl PacketFrame {
    /// Parses the [`PacketFrame`] into a [`ReadPacket`] by consuming the frame. If one of multiple
    /// possible packets should be parsed, use the [`match_packet`] macro instead.
    pub fn into_packet<T: ReadPacket>(self) -> Result<T, Error> {
        if T::ID != self.id {
            return Err(Error::IllegalPacketId(self.id));
        }
        let mut reader = Cursor::new(&self.data);
        let packet = T::read_packet(&mut reader)?;
        Ok(packet)
    }
}

/// [`PacketCodec`] is a codec for reading and writing [`Packet`] from an async reader and writer
/// (e.g., a tokio tcp stream). The codec is mento to be used with a [`tokio_util::codec::Framed`].
/// The framed allows for cancellation safe reads from the underlying async reader. As well as efficient
/// writes to the underlying async writer.
///
/// The codec reads bytes from the underlying stream and produces [`PacketFrame`]s. These are independent
/// of the current protocol phase and whether the codec is used for a client or server. The codec writes
/// typed [`WritePacket`] implementations.
///
/// The decoder also supports encryption and decryption ciphers. If set, they will encrypt and decrypt
/// all incoming and outgoing bytes.
pub struct PacketCodec {
    /// The maximum packet size allowed to be received. Larger packets will close the connection.
    max_packet_size: usize,

    /// An internal write buffer such that the packet length can be written before the packet data.
    write_buffer: BytesMut,

    /// The current position in the source buffer until which bytes have been decrypted.
    decrypted_until: usize,

    /// The cipher pair used for encryption and decryption.
    ciphers: Option<(Aes128Cfb8Enc, Aes128Cfb8Dec)>,
}

impl PacketCodec {
    /// Creates a new [`PacketCodec`] with a maximum allowed packet size.
    pub fn new(max_packet_size: usize) -> Self {
        assert_eq!(
            Aes128Cfb8Dec::block_size(),
            1,
            "The aes-cfb8 block size should be one byte"
        );
        Self {
            max_packet_size,
            write_buffer: BytesMut::new(),
            decrypted_until: 0,
            ciphers: None,
        }
    }

    /// Sets the encryption and decryption ciphers from a shared secret. Generally, this should only
    /// be called once while the codec is unencrypted. Resetting the ciphers is possible but should
    /// only be done while no packets are potentially only partially read.
    pub fn encrypt(&mut self, shared_secret: &[u8]) -> Result<(), InvalidLength> {
        self.ciphers = Some(ciphers(shared_secret)?);
        Ok(())
    }

    /// Whether the codec is currently encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.ciphers.is_some()
    }
}

impl Decoder for PacketCodec {
    type Item = PacketFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Decrypt all new bytes if the ciphers are set. The bytes are decrypted in place on the source
        // buffer. The chunking always aligns as the block size of the decryptor is always one.
        if let Some((_, decryptor)) = self.ciphers.as_mut() {
            let unencrypted = &mut src[self.decrypted_until..];
            for chunk in unencrypted.chunks_mut(Aes128Cfb8Dec::block_size()) {
                let gen_arr = GenericArray::from_mut_slice(chunk);
                decryptor.decrypt_block_mut(gen_arr);
            }
            self.decrypted_until = src.len();
        }

        // Try reading the packet length from the buffer. In the case of an EOF error, read more bytes.
        let mut reader = Cursor::new(&src);
        let length = match reader.read_varint() {
            Ok(length) => length as usize,
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(err) => return Err(err),
        };

        // In case the length is larger than the maximum packet size, return an error.
        if length > self.max_packet_size {
            return Err(Error::IllegalPacketLength);
        }

        // In case the buffer is not large enough, reserve space for the rest of the packet. It needs
        // to read the size of the packet minus the bytes it already read, without including the packet
        // length field.
        let length_len = reader.position() as usize;
        if src.len() < length + length_len {
            src.reserve(length - (src.len() - length_len));
            return Ok(None);
        }

        // Take a view of the packet bytes, not including the packet length field. All previous bytes
        // are dropped. Taking the view is zero-copy, as such unsupported packets entail minimal
        // performance loss.
        let id = reader.read_varint()?;
        let id_len = reader.position() as usize;
        self.decrypted_until = self.decrypted_until.saturating_sub(length_len + length);
        let data = src.split_to(length_len + length).split_off(id_len).freeze();
        Ok(Some(PacketFrame { length, id, data }))
    }
}

impl<T> Encoder<T> for PacketCodec
where
    T: WritePacket,
{
    type Error = Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Write the packet data to a buffer. We cannot write directly to the destination buffer because
        // the packet length is not known yet.
        self.write_buffer.clear();
        let mut writer = (&mut self.write_buffer).writer();
        writer.write_varint(T::ID as VarInt)?;
        item.write_packet(&mut writer)?;

        // Write the packet length, id, and data.
        let encrypted_until = dst.len();
        let mut writer = dst.writer();
        writer.write_varint(self.write_buffer.len() as VarInt)?;
        writer.write_all(&self.write_buffer)?;

        // Encrypt all new bytes if the ciphers are set. The bytes are encrypted in place on the source
        // buffer. The chunking always aligns as the block size of the encryptor is always one.
        if let Some((encryptor, _)) = self.ciphers.as_mut() {
            let unencrypted = &mut dst[encrypted_until..];
            for chunk in unencrypted.chunks_mut(Aes128Cfb8Dec::block_size()) {
                let gen_arr = GenericArray::from_mut_slice(chunk);
                encryptor.encrypt_block_mut(gen_arr);
            }
        }

        Ok(())
    }
}
