use crate::{AsyncWritePacket, Error, INITIAL_BUFFER_SIZE, VarInt, VarLong, WritePacket};
use std::fmt::Debug;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

impl<W: AsyncWrite + Unpin + Send + Sync> AsyncWritePacket for W {
    async fn write_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        // create a new buffer (our packets are very small)
        let mut buffer = Vec::with_capacity(INITIAL_BUFFER_SIZE);

        // write the packets id and the respective packets content
        buffer.write_varint(T::get_packet_id() as VarInt).await?;
        packet.write_to_buffer(&mut buffer).await?;

        // prepare a final buffer (leaving max 2 bytes for varint (packets never get that big))
        let packet_len = buffer.len();
        let mut final_buffer = Vec::with_capacity(packet_len + 2);
        final_buffer.write_varint(packet_len as VarInt).await?;
        final_buffer.extend_from_slice(&buffer);

        // send the final buffer into the stream
        self.write_all(&final_buffer).await?;

        Ok(())
    }

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
        self.write_u8(bool as u8).await?;

        Ok(())
    }

    async fn write_text_component(&mut self, str: &str) -> Result<(), Error> {
        // writes a TAG_String (0x08) TextComponent
        self.write_u8(0x08).await?;
        self.write_u16(str.len() as u16).await?;
        self.write_all(str.as_bytes()).await?;

        Ok(())
    }

    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error> {
        self.write_varint(arr.len() as VarInt).await?;
        self.write_all(arr).await?;

        Ok(())
    }
}
