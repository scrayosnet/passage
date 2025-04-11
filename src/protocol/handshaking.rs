use crate::connection::{Connection, Phase, phase};
use crate::protocol::{AsyncReadPacket, Error, InboundPacket, Packet, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tracing::debug;

pub mod inbound {
    use super::*;

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
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let protocol_version = buffer.read_varint().await? as isize;
            let server_address = buffer.read_string().await?;
            let server_port = buffer.read_u16().await?;
            let next_state = buffer.read_varint().await?.try_into()?;

            Ok(Self {
                protocol_version,
                server_address,
                server_port,
                next_state,
            })
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received handshake packet");
            phase!(con.phase, Phase::Handshake, client_address,);

            // collect information
            let client_address = *client_address;
            let protocol_version = self.protocol_version;
            let server_address = self.server_address.to_string();
            let server_port = self.server_port;
            let transfer = self.next_state == State::Transfer;

            // switch to next phase based on state
            con.phase = match &self.next_state {
                State::Status => Phase::Status {
                    client_address,
                    server_address,
                    server_port,
                    protocol_version,
                },
                _ => Phase::Login {
                    client_address,
                    server_address,
                    server_port,
                    protocol_version,
                    transfer,
                },
            };

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncWritePacket;
    use std::io::Cursor;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(inbound::HandshakePacket::get_packet_id(), 0x00);
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

        let mut read_buffer: Cursor<Vec<u8>> = Cursor::new(buffer.into_inner());
        let packet = inbound::HandshakePacket::new_from_buffer(&mut read_buffer)
            .await
            .unwrap();

        assert_eq!(packet.protocol_version, protocol_version as isize);
        assert_eq!(packet.server_address, server_address);
        assert_eq!(packet.server_port, server_port);
        assert_eq!(packet.next_state, next_state);

        assert_eq!(
            read_buffer.position() as usize,
            read_buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
