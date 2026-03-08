use crate::handshake;
use crate::packet::DynPacket;
use crate::reader::{ReadPacket, ReadPacketExt};
use crate::writer::{WritePacket, WritePacketExt};
use crate::{Error, Packet, VarInt};
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, BlockSizeUser, InvalidLength};
use bytes::BufMut;
use cfb8::cipher::KeyIvInit;
use futures::SinkExt;
use std::io::{Cursor, Write};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::StreamExt;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, Framed};

pub struct Connection<S> {
    stream: Framed<S, PacketCodec>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    pub fn new(stream: S, max_packet_size: usize) -> Self {
        Self { stream: Framed::new(stream, PacketCodec::new_server(max_packet_size)) }
    }

    pub async fn read(&mut self) -> Result<DynPacket, Error> {
        // TODO add tracing span here
        self.stream.next().await
            // TODO add own error type here
            .ok_or(Error::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Connection closed")))?
    }

    pub async fn write<T: WritePacket>(&mut self, packet: T) -> Result<(), Error> {
        // TODO add tracing span here
        self.stream.send(packet).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Handshake,
    Status,
    Login,
    Play,
}

pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

/// Creates a cipher pair for encrypting a TCP stream. The pair is synced for the same shared secret.
pub fn ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), InvalidLength> {
    let encryptor = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decryptor = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;
    Ok((encryptor, decryptor))
}

// TODO encryption and phase enum?
pub struct PacketCodec {
    max_packet_size: usize,
    write_buffer: BytesMut,
    decrypted_until: usize,
    ciphers: Option<(Aes128Cfb8Enc, Aes128Cfb8Dec)>,
    server: bool,
    phase: Phase,
}

impl PacketCodec {
    pub fn new_server(max_packet_size: usize) -> Self {
        assert_eq!(Aes128Cfb8Dec::block_size(), 1, "The aes-cfb8 block size should be one byte");
        Self { max_packet_size, write_buffer: BytesMut::new(), decrypted_until: 0, ciphers: None, server: true, phase: Phase::Handshake }
    }

    pub fn new_client(max_packet_size: usize) -> Self {
        assert_eq!(Aes128Cfb8Enc::block_size(), 1, "The aes-cfb8 block size should be one byte");
        Self { max_packet_size, write_buffer: BytesMut::new(), decrypted_until: 0, ciphers: None, server: false, phase: Phase::Handshake }
    }

    pub fn encrypt(&mut self, shared_secret: &[u8]) -> Result<(), InvalidLength> {
        self.ciphers = Some(ciphers(shared_secret)?);
        Ok(())
    }

    pub fn is_encrypted(&self) -> bool {
        self.ciphers.is_some()
    }

    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn set_server(&mut self, server: bool) {
        self.server = server;
    }

    pub fn is_server(&self) -> bool {
        self.server
    }
}

impl Decoder for PacketCodec {
    type Item = DynPacket;
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
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(err) => return Err(err),
        };

        // In case the length is larger than the maximum packet size, return an error.
        if length > self.max_packet_size {
            return Err(Error::IllegalPacketLength)
        }

        // In case the buffer is not large enough, reserve space for the rest of the packet. It needs
        // to read the size of the packet minus the bytes it already read, without including the packet
        // length field.
        let length_len = reader.position() as usize;
        if (src.len() - length_len) < length {
            src.reserve(length - (src.len() - length_len));
            return Ok(None);
        }

        // Try reading the packet id from the buffer. There should be enough bytes to parse the whole packet.
        // First read the packet id and then create a buffer that holds just the
        let mut reader = {
            use std::io::Read;
            reader.take(length as u64)
        };
        let id = reader.read_varint()?;
        let packet = match (self.server, self.phase, id) {
            // TODO add serverbound and phase as const to type? Then define this match with a macro?
            (true, Phase::Handshake, handshake::serverbound::HandshakePacket::ID) => DynPacket::Handshake(handshake::serverbound::HandshakePacket::read_packet(&mut reader)?),
            // TODO update error
            _ => return Err(Error::IllegalPacketId { expected: id, actual: id }),
        };

        // Update the source buffer to the position after the packet data and decryption position.
        {
            use bytes::Buf;
            src.advance(length + length_len);
            self.decrypted_until = self.decrypted_until.saturating_sub(length + length_len);
        }

        Ok(Some(packet))
    }
}

// TODO implement for dyn packet?
impl <T> Encoder<T> for PacketCodec where T: WritePacket {
    type Error = Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Write the packet data to a buffer. We cannot write directly to the destination buffer because
        // the packet length is not known yet.
        self.write_buffer.clear();
        let mut writer = (&mut self.write_buffer).writer();
        item.write_packet(&mut writer)?;

        // Write the packet length, id, and data.
        let encrypted_until = dst.len();
        let mut writer = dst.writer();
        writer.write_varint(self.write_buffer.len() as VarInt)?;
        writer.write_varint(T::ID as VarInt)?;
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
