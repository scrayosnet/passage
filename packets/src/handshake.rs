#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::{Packet, State, VarInt};
#[cfg(test)]
use fake::Dummy;

pub mod serverbound {
    use super::*;
    #[cfg(feature = "server")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "client")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(feature = "server")]
    use tokio::io::{AsyncRead, AsyncReadExt};
    #[cfg(feature = "client")]
    use tokio::io::{AsyncWrite, AsyncWriteExt};

    /// The [`HandshakePacket`].
    ///
    /// This packet causes the server to switch into the target state. It should be sent right after
    /// opening the TCP connection to prevent the server from disconnecting.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Handshake)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct HandshakePacket {
        /// The pretended protocol version.
        pub protocol_version: VarInt,
        /// The pretended server address.
        pub server_address: String,
        /// The pretended server port.
        pub server_port: u16,
        /// The protocol states to initiate.
        pub next_state: State,
    }

    impl Packet for HandshakePacket {
        fn get_packet_id() -> VarInt {
            0x00
        }
    }

    #[cfg(feature = "client")]
    impl WritePacket for HandshakePacket {
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

    #[cfg(feature = "server")]
    impl ReadPacket for HandshakePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_packet;

    #[tokio::test]
    async fn write_read_serverbound_handshake_packet() {
        assert_packet::<serverbound::HandshakePacket>(0x00).await;
    }
}
