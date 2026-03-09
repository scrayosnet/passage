use crate::{Error, Packet, VarInt, VarLong};
pub use byteorder::{BigEndian, ReadBytesExt};
use fastnbt::{DeOpts, Value};
pub use std::io::Read;
use uuid::Uuid;

pub trait ReadPacket: Packet + Sized {
    fn read_packet(src: &mut impl Read) -> Result<Self, Error>;
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