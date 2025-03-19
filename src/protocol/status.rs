use crate::protocol::{AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// This packet requests the server metadata for display in the multiplayer menu.
///
/// The status can be hidden by closing the connection instead. After the status was exchanged, a ping sequence may
/// be performed afterward.
#[derive(Debug)]
pub struct StatusRequestPacket;

impl Packet for StatusRequestPacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl InboundPacket for StatusRequestPacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
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
    async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        let payload = buffer.read_u64().await?;

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
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_string(&self.body).await?;

        Ok(())
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
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        buffer.write_u64(self.payload).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncReadPacket;
    use std::io::Cursor;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(StatusRequestPacket::get_packet_id(), 0x00);
        assert_eq!(PingPacket::get_packet_id(), 0x01);
        assert_eq!(StatusResponsePacket::get_packet_id(), 0x00);
        assert_eq!(PongPacket::get_packet_id(), 0x01);
    }

    #[tokio::test]
    async fn decode_status_request() {
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        let _packet = StatusRequestPacket::new_from_buffer(&mut buffer)
            .await
            .unwrap();
        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn decode_ping() {
        let payload = 11u64;

        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        buffer.write_u64(payload).await.unwrap();
        let packet = PingPacket::new_from_buffer(&mut buffer).await.unwrap();
        assert_eq!(packet.payload, payload);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn encode_status_response() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = StatusResponsePacket::new("{\"some\": \"values\"}".to_string());
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

        let body = buffer.read_string().await.unwrap();
        assert_eq!(body, packet.body);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn encode_pong() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = PongPacket::new(17);
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

        let payload = buffer.read_u64().await.unwrap();
        assert_eq!(payload, packet.payload);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
