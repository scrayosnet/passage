use crate::protocol::{AsyncReadPacket, Error, InboundPacket, Packet, State};
use std::io::Cursor;
use tokio::io::AsyncReadExt;

/// This packet initiates the status request attempt and tells the server the details of the client.
///
/// The data in this packet can differ from the actual data that was used but will be considered by the server when
/// assembling the response. Therefore, this data should mirror what a normal client would send.
#[derive(Debug)]
pub struct HandshakePacket {
    /// The pretended protocol version.
    pub protocol_version: isize,
    /// The pretended server address.
    pub server_address: String,
    /// The pretended server port.
    pub server_port: u16,
    /// The protocol state to initiate.
    pub next_state: State,
}

impl Packet for HandshakePacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl InboundPacket for HandshakePacket {
    async fn new_from_buffer(buffer: &[u8]) -> Result<Self, Error> {
        let mut reader = Cursor::new(buffer);

        let protocol_version = reader.read_varint().await? as isize;
        let server_address = reader.read_string().await?;
        let server_port = reader.read_u16().await?;
        let next_state = reader.read_varint().await?.try_into()?;

        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }
}
