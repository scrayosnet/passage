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
