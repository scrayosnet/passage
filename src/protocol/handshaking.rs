use crate::protocol::{AsyncReadPacket, Error, InboundPacket, Packet, State};
use std::io::Cursor;
use tokio::io::AsyncReadExt;

/// This packet initiates the connection and tells the server the details of the client and intent.
///
/// The data in this packet can differ from the actual data that was used but will be considered by the server when
/// assembling the response. Therefore, these values can be assumed as true.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncWritePacket;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(HandshakePacket::get_packet_id(), 0x00);
    }

    #[tokio::test]
    async fn decode_handshake() {
        let protocol_version = 13;
        let server_address = "test";
        let server_port = 1337;
        let next_state = State::Transfer;

        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        buffer.write_varint(protocol_version).await.unwrap();
        buffer.write_string(server_address).await.unwrap();
        buffer.write_u16(server_port).await.unwrap();
        buffer.write_varint(next_state.into()).await.unwrap();

        let packet = HandshakePacket::new_from_buffer(&buffer.get_ref().clone())
            .await
            .unwrap();
        assert_eq!(packet.protocol_version, protocol_version as isize);
        assert_eq!(packet.server_address, server_address);
        assert_eq!(packet.server_port, server_port);
        assert_eq!(packet.next_state, next_state);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
