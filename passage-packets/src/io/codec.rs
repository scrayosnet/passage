use crate::io::reader::{ReadPacket, ReadPacketExt};
use crate::io::writer::{WritePacket, WritePacketExt};
use crate::{Error, VarInt};
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, BlockSizeUser, InvalidLength};
use bytes::{BufMut, Bytes};
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
        Self { stream: Framed::new(stream, PacketCodec::new(max_packet_size)) }
    }

    pub async fn read<T: ReadPacket>(&mut self) -> Result<T, Error> {
        // TODO add tracing span here
        Ok(self.stream.next().await.expect("Connection closed")?.into_packet()?)
    }

    pub async fn write<T: WritePacket>(&mut self, packet: T) -> Result<(), Error> {
        // TODO add tracing span here
        self.stream.send(packet).await?;
        Ok(())
    }
}

pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

/// Creates a cipher pair for encrypting a TCP stream. The pair is synced for the same shared secret.
pub fn ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), InvalidLength> {
    let encryptor = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decryptor = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;
    Ok((encryptor, decryptor))
}

pub struct PacketFrame {
    pub length: usize,
    pub id: VarInt,
    pub data: Bytes,
}

impl PacketFrame {
    pub fn into_packet<T: ReadPacket>(self) -> Result<T, Error> {
        let mut reader = Cursor::new(&self.data);
        let packet = T::read_packet(&mut reader)?;
        Ok(packet)
    }
}

pub struct PacketCodec {
    max_packet_size: usize,
    write_buffer: BytesMut,
    decrypted_until: usize,
    ciphers: Option<(Aes128Cfb8Enc, Aes128Cfb8Dec)>,
}

impl PacketCodec {
    pub fn new(max_packet_size: usize) -> Self {
        assert_eq!(Aes128Cfb8Dec::block_size(), 1, "The aes-cfb8 block size should be one byte");
        Self { max_packet_size, write_buffer: BytesMut::new(), decrypted_until: 0, ciphers: None }
    }

    pub fn encrypt(&mut self, shared_secret: &[u8]) -> Result<(), InvalidLength> {
        self.ciphers = Some(ciphers(shared_secret)?);
        Ok(())
    }

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

        // Take a view of the packet bytes, not including the packet length field. All previous bytes
        // are dropped. Taking the view is zero-copy, as such unsupported packets entail minimal
        // performance loss.
        let id = reader.read_varint()?;
        self.decrypted_until = self.decrypted_until.saturating_sub(reader.position() as usize);
        let data = src.split_to(reader.position() as usize).freeze();
        Ok(Some(PacketFrame { length, id, data }))
    }
}

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
