use crate::{AsyncWritePacket, Error, VarInt, VarLong};
use fastnbt::SerOpts;
use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

impl<W: AsyncWrite + Unpin + Send + Sync> AsyncWritePacket for W {
    async fn write_varint(&mut self, value: VarInt) -> Result<(), Error> {
        let mut value = value;
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i32::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf).await?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    async fn write_varlong(&mut self, value: VarLong) -> Result<(), Error> {
        let mut value = value;
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i64::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf).await?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    async fn write_string(&mut self, string: &str) -> Result<(), Error> {
        self.write_varint(string.len() as VarInt).await?;
        self.write_all(string.as_bytes()).await?;

        Ok(())
    }

    async fn write_uuid(&mut self, id: &Uuid) -> Result<(), Error> {
        self.write_u128(id.as_u128()).await?;

        Ok(())
    }

    async fn write_bool(&mut self, bool: bool) -> Result<(), Error> {
        self.write_u8(u8::from(bool)).await?;

        Ok(())
    }

    async fn write_text_component(&mut self, str: &str) -> Result<(), Error> {
        if !str.starts_with('{') {
            // writes a TAG_String (0x08) TextComponent
            self.write_u8(0x08).await?;
            self.write_u16(str.len() as u16).await?;
            self.write_all(str.as_bytes()).await?;
            return Ok(());
        }

        // writes a TAG_Compound (0x0a) TextComponent
        let json: Value = serde_json::from_str(str)?;
        let bytes = fastnbt::to_bytes_with_opts(&json, SerOpts::network_nbt())?;
        self.write_all(&bytes).await?;

        Ok(())
    }

    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error> {
        self.write_varint(arr.len() as VarInt).await?;
        self.write_all(arr).await?;

        Ok(())
    }
}
