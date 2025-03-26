use crate::connection::Connection;
use crate::protocol::Error::Generic;
use crate::protocol::{AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet, Phase};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug)]
pub struct TransferPacket {
    host: String,
    port: usize,
}

impl TransferPacket {
    pub const fn new(host: String, port: usize) -> Self {
        Self { host, port }
    }

    pub fn from_addr(addr: impl Into<SocketAddr>) -> Self {
        let addr = addr.into();
        Self {
            host: addr.ip().to_string(),
            port: addr.port() as usize,
        }
    }
}

impl Packet for TransferPacket {
    fn get_packet_id() -> usize {
        0x0B
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl OutboundPacket for TransferPacket {
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_string(&self.host).await?;
        buffer.write_varint(self.port).await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct DisconnectPacket {
    /// The JSON response reason that contains all self-reported server metadata.
    reason: String,
}

impl Packet for DisconnectPacket {
    fn get_packet_id() -> usize {
        0x02
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl OutboundPacket for DisconnectPacket {
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_string(&self.reason).await?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct KeepAlivePacket {
    id: u64,
}

impl KeepAlivePacket {
    pub const fn new(id: u64) -> Self {
        Self { id }
    }
}

impl Packet for KeepAlivePacket {
    fn get_packet_id() -> usize {
        0x04
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl InboundPacket for KeepAlivePacket {
    async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        let id = buffer.read_u64().await?;
        Ok(Self { id })
    }

    async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        if !con.last_keep_alive.replace(self.id, 0) {
            return Err(Generic("keep alive packet already received".to_string()));
        }

        Ok(())
    }
}

impl OutboundPacket for KeepAlivePacket {
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_u64(self.id).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncReadPacket;
    use std::io::Cursor;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(TransferPacket::get_packet_id(), 0x0B);
    }

    #[tokio::test]
    async fn decode_handshake() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = TransferPacket::new("test".to_string(), 1337);
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

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
