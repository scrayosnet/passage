use crate::{AsyncReadPacket, Error, ReadPacket, VarInt, VarLong};
use fastnbt::{DeOpts, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use uuid::Uuid;

impl<R: AsyncRead + Unpin + Send + Sync> AsyncReadPacket for R {
    async fn read_packet<T: ReadPacket + Send + Sync>(&mut self) -> Result<T, Error> {
        // extract the length of the packets and check for any following content
        let length = self.read_varint().await?;
        if length == 0 || length > 10_000 {
            return Err(Error::IllegalPacketLength);
        }

        // extract the encoded packets id and validate if it is expected
        let packet_id = self.read_varint().await?;
        let expected_packet_id = T::ID;
        if packet_id != expected_packet_id {
            return Err(Error::IllegalPacketId {
                expected: expected_packet_id,
                actual: packet_id,
            });
        }

        // split a separate reader from the stream
        let mut take = self.take(length as u64);

        // convert the received buffer into our expected packets
        T::read_from_buffer(&mut take).await
    }

    async fn read_varint(&mut self) -> Result<VarInt, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..5 {
            self.read_exact(&mut buf).await?;
            ans |= ((buf[0] & 0b0111_1111) as i32) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    async fn read_varlong(&mut self) -> Result<VarLong, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..9 {
            self.read_exact(&mut buf).await?;
            ans |= ((buf[0] & 0b0111_1111) as i64) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    async fn read_string(&mut self) -> Result<String, Error> {
        let length = self.read_varint().await? as usize;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding)
    }

    async fn read_bool(&mut self) -> Result<bool, Error> {
        let bool = self.read_u8().await?;
        Ok(bool == 1u8)
    }

    async fn read_uuid(&mut self) -> Result<Uuid, Error> {
        let value = self.read_u128().await?;

        Ok(Uuid::from_u128(value))
    }

    async fn read_text_component(&mut self) -> Result<String, Error> {
        let tag = self.read_u8().await?;
        if tag == 0x08 {
            // expect a TAG_String (0x08) TextComponent
            let len = self.read_u16().await?;

            let mut buffer = vec![0; len as usize];
            self.read_exact(&mut buffer).await?;

            return String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding);
        }

        // TODO reads endlessly?
        // expect it to take the full buffer
        let mut buffer = vec![];
        self.read_to_end(&mut buffer).await?;
        let nbt: Value = fastnbt::from_bytes_with_opts(&buffer, DeOpts::network_nbt())?;
        let json: String = serde_json::to_string(&nbt)?;

        Ok(json)
    }

    async fn read_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let length = self.read_varint().await? as usize;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        Ok(buffer)
    }
}
