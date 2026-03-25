use crate::{Error, Packet, VarInt, VarLong};
pub use byteorder::{BigEndian, WriteBytesExt};
use fastnbt::{SerOpts, Value};
pub use std::io::Write;
use uuid::Uuid;

/// A [`WritePacket`] is a [`Packet`] that can be written to a [`Write`].
pub trait WritePacket: Packet {
    /// Writes a [`Packet`] to a [`Write`]. It writes neither the packet length nor the packet id.
    fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error>;
}

/// A collection of utilities for writing packet related data to a [`Write`].
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

impl<T> WritePacketExt for T
where
    T: Write,
{
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
