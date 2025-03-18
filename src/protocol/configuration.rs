use crate::protocol::{AsyncWritePacket, Error, OutboundPacket, Packet};
use std::io::Cursor;

#[derive(Debug)]
pub struct TransferPacket {
    host: String,
    port: usize,
}

impl TransferPacket {
    pub const fn new(host: String, port: usize) -> Self {
        Self { host, port }
    }
}

impl Packet for TransferPacket {
    fn get_packet_id() -> usize {
        0x0B
    }
}

impl OutboundPacket for TransferPacket {
    async fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::<u8>::new());

        buffer.write_string(&self.host).await?;
        buffer.write_varint(self.port).await?;

        Ok(buffer.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncReadPacket;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(TransferPacket::get_packet_id(), 0x0B);
    }

    #[tokio::test]
    async fn decode_handshake() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = TransferPacket::new("test".to_string(), 1337);
        let packet_buffer = packet.to_buffer().await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer);

        let host = buffer.read_string().await.unwrap();
        let port = buffer.read_varint().await.unwrap();
        assert_eq!(host, packet.host);
        assert_eq!(port, packet.port);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
