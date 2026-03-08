use crate::{Error, Packet, VarInt, VarLong};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use bytes::{Buf, BufMut};
use fastnbt::{DeOpts, SerOpts, Value};
use std::io::{Cursor, Read, Write};
use futures::SinkExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::StreamExt;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, Framed};
use uuid::Uuid;

pub struct Connection<S> {
    stream: Framed<S, PacketCodec>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Connection<S> {
    pub fn new(stream: S, max_packet_size: usize) -> Self {
        Self { stream: Framed::new(stream, PacketCodec::new(max_packet_size)) }
    }

    pub async fn read<T: ReadPacket>(&mut self) -> Result<T, Error> {
        self.stream.next().await.expect("or connection closed")?.into_packet()
    }

    pub async fn write<T: WritePacket>(&mut self, packet: T) -> Result<(), Error> {
        self.stream.send(packet).await?;
        Ok(())
    }
}

pub async fn decode_packet<T: ReadPacket, S: AsyncRead + Unpin>(stream: S, max_packet_size: usize) -> Result<T, Error> {
    let mut frames = Framed::new(stream, PacketCodec::new(max_packet_size));
    frames.next().await.unwrap()?.into_packet()
}

pub struct PacketFrame {
    pub length: VarInt,
    pub id: VarInt,
    pub data: Vec<u8>,
}

impl PacketFrame {
    pub fn into_packet<T: ReadPacket>(self) -> Result<T, Error> {
        if self.id != T::ID {
            return Err(Error::IllegalPacketId {
                // FIXME which way is correct?
                expected: T::ID,
                actual: self.id as VarInt,
            })
        }
        let packet = T::read_packet(&mut self.data.reader())?;
        Ok(packet)
    }
}

// TODO add decrypt buffer that 

// TODO encryption and phase enum?
pub struct PacketCodec {
    max_packet_size: usize,
    write_buffer: BytesMut
}

impl PacketCodec {
    pub fn new(max_packet_size: usize) -> Self {
        Self { max_packet_size, write_buffer: BytesMut::new() }
    }

    fn try_decode(&self, reader: &mut impl Read) -> Result<PacketFrame, Error> {
        // read the packet length
        let packet_length = reader.read_varint()?;
        if packet_length < 0 || packet_length as usize > self.max_packet_size {
            return Err(Error::IllegalPacketLength);
        }

        // read packet data
        let packet_id = reader.read_varint()?;
        // TODO can we prevent creating a new vec for each packet?
        let mut packet_data = vec![0; packet_length as usize];
        reader.read_exact(&mut packet_data)?;

        Ok(PacketFrame {
            length: packet_length,
            id: packet_id,
            data: packet_data,
        })
    }
}

impl Decoder for PacketCodec {
    type Item = PacketFrame;
    type Error = Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Try to deserialize the packet from the current buffer.
        let mut reader = Cursor::new(&src);
        // TODO use src.split_to instead?
        match self.try_decode(&mut reader) {
            // The packet was successfully deserialized, advance the buffer and return the packet.
            Ok(packet) => {
                src.advance(reader.position() as usize);
                Ok(Some(packet))
            }
            // The packet was not fully deserialized yet, return None.
            Err(Error::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                // TODO reserve space in buffer!
                Ok(None)
            }
            // Some other error occurred, return it.
            Err(err) => {
                Err(err)
            }
        }
    }
}

impl <T> Encoder<T> for PacketCodec where T: WritePacket {
    type Error = Error;

    fn encode(&mut self, item: T, dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Write the packet data to a buffer
        self.write_buffer.clear();
        let mut writer = (&mut self.write_buffer).writer();
        item.write_packet(&mut writer)?;

        // Write the packet length, id, and data
        let mut writer = dst.writer();
        writer.write_varint(self.write_buffer.len() as VarInt)?;
        writer.write_varint(T::ID as VarInt)?;
        writer.write_all(&self.write_buffer)?;
        Ok(())
    }
}

pub trait WritePacket: Packet {
    fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error>;
}

pub trait ReadPacket: Packet + Sized {
    fn read_packet(src: &mut impl Read) -> Result<Self, Error>;
}

pub trait WritePacketExt {
    /// Writes a [`VarInt`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn write_varint(&mut self, value: VarInt) -> Result<(), Error>;

    /// Writes a [`VarLong`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn write_varlong(&mut self, value: VarLong) -> Result<(), Error>;

    /// Writes a `String` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    fn write_string(&mut self, value: &str) -> Result<(), Error>;

    /// Writes a `Uuid` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    fn write_uuid(&mut self, value: &Uuid) -> Result<(), Error>;

    /// Writes a `bool` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    fn write_bool(&mut self, value: bool) -> Result<(), Error>;

    /// Writes a string `TextComponent` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol/Packets#Type:Text_Component
    fn write_text_component(&mut self, value: &str) -> Result<(), Error>;

    /// Writes a vec of `u8` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    fn write_bytes(&mut self, value: &[u8]) -> Result<(), Error>;
}

pub trait ReadPacketExt {
    /// Reads a [`VarInt`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn read_varint(&mut self) -> Result<VarInt, Error>;

    /// Reads a [`VarLong`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn read_varlong(&mut self) -> Result<VarLong, Error>;

    /// Reads a `String` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    fn read_string(&mut self) -> Result<String, Error>;

    /// Reads a `bool` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    fn read_bool(&mut self) -> Result<bool, Error>;

    /// Reads a `Uuid` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    fn read_uuid(&mut self) -> Result<Uuid, Error>;

    /// Reads a string `TextComponent` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol/Packets#Type:Text_Component
    fn read_text_component(&mut self) -> Result<String, Error>;

    /// Reads a vec of `u8` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    fn read_bytes(&mut self) -> Result<Vec<u8>, Error>;
}

impl <T> WritePacketExt for T where T: Write {
    fn write_varint(&mut self, mut value: VarInt) -> Result<(), Error> {
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i32::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf)?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    fn write_varlong(&mut self, mut value: VarLong) -> Result<(), Error> {
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i64::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf)?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    fn write_string(&mut self, value: &str) -> Result<(), Error> {
        self.write_varint(value.len() as VarInt)?;
        self.write_all(value.as_bytes())?;
        Ok(())
    }

    fn write_uuid(&mut self, value: &Uuid) -> Result<(), Error> {
        self.write_u128::<BigEndian>(value.as_u128())?;
        Ok(())
    }

    fn write_bool(&mut self, value: bool) -> Result<(), Error> {
        self.write_u8(u8::from(value))?;
        Ok(())
    }

    fn write_text_component(&mut self, value: &str) -> Result<(), Error> {
        if !value.starts_with('{') {
            // writes a TAG_String (0x08) TextComponent
            self.write_u8(0x08)?;
            self.write_u16::<BigEndian>(value.len() as u16)?;
            self.write_all(value.as_bytes())?;
            return Ok(());
        }

        // writes a TAG_Compound (0x0a) TextComponent
        let json: Value = serde_json::from_str(value)?;
        let bytes = fastnbt::to_bytes_with_opts(&json, SerOpts::network_nbt())?;
        self.write_all(&bytes)?;

        Ok(())
    }

    fn write_bytes(&mut self, value: &[u8]) -> Result<(), Error> {
        self.write_varint(value.len() as VarInt)?;
        self.write_all(value)?;
        Ok(())
    }
}

impl <T> ReadPacketExt for T where T: Read {
    fn read_varint(&mut self) -> Result<VarInt, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..5 {
            self.read_exact(&mut buf)?;
            ans |= (i32::from(buf[0] & 0b0111_1111)) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    fn read_varlong(&mut self) -> Result<VarLong, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..9 {
            self.read_exact(&mut buf)?;
            ans |= (i64::from(buf[0] & 0b0111_1111)) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    fn read_string(&mut self) -> Result<String, Error> {
        let length = self.read_varint()? as usize;
        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer)?;
        String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding)
    }

    fn read_bool(&mut self) -> Result<bool, Error> {
        let bool = self.read_u8()?;
        Ok(bool == 1u8)
    }

    fn read_uuid(&mut self) -> Result<Uuid, Error> {
        let value = self.read_u128::<BigEndian>()?;
        Ok(Uuid::from_u128(value))
    }

    fn read_text_component(&mut self) -> Result<String, Error> {
        let tag = self.read_u8()?;
        if tag == 0x08 {
            // expect a TAG_String (0x08) TextComponent
            let len = self.read_u16::<BigEndian>()?;
            let mut buffer = vec![0; len as usize];
            self.read_exact(&mut buffer)?;
            return String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding);
        }

        // expect it to take the full buffer (the text component is the last element in the packet)
        let mut buffer = vec![tag];
        self.read_to_end(&mut buffer)?;
        let nbt: Value = fastnbt::from_bytes_with_opts(&buffer, DeOpts::network_nbt())?;
        let json: String = serde_json::to_string(&nbt)?;
        Ok(json)
    }

    fn read_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let length = self.read_varint()? as usize;
        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer)?;
        Ok(buffer)
    }
}
