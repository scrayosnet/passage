use crate::connection::{Connection, Phase, phase};
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet, State,
};
use fake::Dummy;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::debug;

pub mod inbound {
    use super::*;
    use crate::protocol::VarInt;
    use crate::status::Protocol;

    /// The inbound [`HandshakePacket`].
    ///
    /// This packet causes the server to switch into the target state. It should be sent right after
    /// opening the TCP connection to prevent the server from disconnecting.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Handshake)
    #[derive(Debug, Clone, Eq, PartialEq, Dummy)]
    pub struct HandshakePacket {
        /// The pretended protocol version.
        pub protocol_version: VarInt,
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

    #[cfg(test)]
    impl OutboundPacket for HandshakePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_varint(self.protocol_version).await?;
            buffer.write_string(&self.server_address).await?;
            buffer.write_u16(self.server_port).await?;
            buffer.write_varint(self.next_state.into()).await?;

            Ok(())
        }
    }

    impl InboundPacket for HandshakePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let protocol_version = buffer.read_varint().await?;
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
    use crate::protocol::tests::assert_packet;

    #[tokio::test]
    async fn packets() {
        assert_packet::<inbound::HandshakePacket>(0x00).await;
    }
}
