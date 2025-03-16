use crate::protocol::{AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet};
use std::io::Cursor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// This packet will be sent after the [`HandshakePacket`] and requests the server metadata.
///
/// The packet can only be sent after the [`HandshakePacket`] and must be written before any status information can be
/// read, as this is the differentiator between the status and the ping sequence.
#[derive(Debug)]
pub struct StatusRequestPacket;

impl Packet for StatusRequestPacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl InboundPacket for StatusRequestPacket {
    async fn new_from_buffer(_buffer: &[u8]) -> Result<Self, Error> {
        Ok(Self)
    }
}

/// This is the request for a specific [`PongPacket`] that can be used to measure the server ping.
///
/// This packet can be sent after a connection was established or the [`StatusResponsePacket`] was received. Initiating
/// the ping sequence will consume the connection after the [`PongPacket`] was received.
#[derive(Debug)]
pub struct PingPacket {
    /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
    pub payload: u64,
}

impl Packet for PingPacket {
    fn get_packet_id() -> usize {
        0x01
    }
}

impl InboundPacket for PingPacket {
    async fn new_from_buffer(buffer: &[u8]) -> Result<Self, Error> {
        let mut reader = Cursor::new(buffer);

        let payload = reader.read_u64().await?;

        Ok(Self { payload })
    }
}

/// This is the response for a specific [`StatusRequestPacket`] that contains all self-reported metadata.
///
/// This packet can be received only after a [`StatusRequestPacket`] and will not close the connection, allowing for a
/// ping sequence to be exchanged afterward.
#[derive(Debug)]
pub struct StatusResponsePacket {
    /// The JSON response body that contains all self-reported server metadata.
    body: String,
}

impl StatusResponsePacket {
    /// Creates a new [`StatusResponsePacket`] with the supplied payload.
    pub const fn new(body: String) -> Self {
        Self { body }
    }
}

impl Packet for StatusResponsePacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl OutboundPacket for StatusResponsePacket {
    async fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::<u8>::new());

        buffer.write_string(&self.body).await?;

        Ok(buffer.into_inner())
    }
}

/// This is the response to a specific [`PingPacket`] that can be used to measure the server ping.
///
/// This packet will be sent after a corresponding [`PingPacket`] and will have the same payload as the request. This
/// also consumes the connection, ending the Server List Ping sequence.
#[derive(Debug)]
pub struct PongPacket {
    /// The arbitrary payload that was sent from the client (to identify the corresponding response).
    payload: u64,
}

impl PongPacket {
    /// Creates a new [`PongPacket`] with the supplied payload.
    pub const fn new(payload: u64) -> Self {
        Self { payload }
    }
}

impl Packet for PongPacket {
    fn get_packet_id() -> usize {
        0x01
    }
}

impl OutboundPacket for PongPacket {
    async fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::<u8>::new());

        buffer.write_u64(self.payload).await?;

        Ok(buffer.into_inner())
    }
}
