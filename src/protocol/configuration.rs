use crate::connection::Connection;
use crate::protocol::Error::Generic;
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet, Phase,
};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

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
pub struct FinishConfigurationPacket;

impl Packet for FinishConfigurationPacket {
    fn get_packet_id() -> usize {
        0x03
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl OutboundPacket for FinishConfigurationPacket {
    async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
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

#[derive(Debug)]
pub struct AddResourcePackPacket {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    /// The JSON response message.
    pub prompt_message: String,
}

impl Packet for AddResourcePackPacket {
    fn get_packet_id() -> usize {
        0x09
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl OutboundPacket for AddResourcePackPacket {
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_uuid(&self.uuid).await?;
        buffer.write_string(&self.url).await?;
        buffer.write_string(&self.hash).await?;
        buffer.write_bool(self.forced).await?;
        buffer.write_string(&self.prompt_message).await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct ResourcePackResponsePacket {
    pub uuid: Uuid,
    pub result: ResourcePackResult,
}

#[derive(Debug)]
enum ResourcePackResult {
    Success,
    Declined,
    DownloadFailed,
    Accepted,
    Downloaded,
    InvalidUrl,
    ReloadFailed,
    Discorded,
}

impl From<ResourcePackResult> for usize {
    fn from(result: ResourcePackResult) -> Self {
        match result {
            ResourcePackResult::Success => 0,
            ResourcePackResult::Declined => 1,
            ResourcePackResult::DownloadFailed => 2,
            ResourcePackResult::Accepted => 3,
            ResourcePackResult::Downloaded => 4,
            ResourcePackResult::InvalidUrl => 5,
            ResourcePackResult::ReloadFailed => 6,
            ResourcePackResult::Discorded => 7,
        }
    }
}

impl TryFrom<usize> for ResourcePackResult {
    type Error = Error;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ResourcePackResult::Success),
            1 => Ok(ResourcePackResult::Declined),
            2 => Ok(ResourcePackResult::DownloadFailed),
            3 => Ok(ResourcePackResult::Accepted),
            4 => Ok(ResourcePackResult::Downloaded),
            5 => Ok(ResourcePackResult::InvalidUrl),
            6 => Ok(ResourcePackResult::ReloadFailed),
            7 => Ok(ResourcePackResult::Discorded),
            _ => Err(Error::IllegalEnumValue { value }),
        }
    }
}

impl Packet for ResourcePackResponsePacket {
    fn get_packet_id() -> usize {
        0x06
    }

    fn get_phase() -> Phase {
        Phase::Configuration
    }
}

impl InboundPacket for ResourcePackResponsePacket {
    async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        let uuid = buffer.read_uuid().await?;
        let result = buffer.read_varint().await?.try_into()?;

        Ok(Self { uuid, result })
    }

    async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        // check state for any final state in the resource pack loading process
        let success = match self.result {
            ResourcePackResult::Success => true,
            ResourcePackResult::Declined
            | ResourcePackResult::DownloadFailed
            | ResourcePackResult::InvalidUrl
            | ResourcePackResult::ReloadFailed
            | ResourcePackResult::Discorded => false,
            _ => {
                // pending state, keep waiting
                return Ok(());
            }
        };

        // get and check internal state
        let Some(configuration) = &mut con.configuration else {
            return Err(Error::Generic("invalid state".to_string()));
        };

        // pop pack from list (ignoring unknown pack ids)
        let Some(pos) = configuration
            .transit_packs
            .iter()
            .position(|(uuid, _)| uuid == &self.uuid)
        else {
            return Ok(());
        };
        let (_, forced) = configuration.transit_packs.swap_remove(pos);

        // handle pack forced
        if forced && !success {
            return Err(Generic("resource pack failed".to_string()));
        }

        // handle all packs transferred
        if configuration.transit_packs.is_empty() {
            return con.transfer().await;
        }

        Ok(())
    }
}

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
