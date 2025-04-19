#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::Packet;
use crate::VarInt;
#[cfg(test)]
use fake::Dummy;

pub mod clientbound {
    use super::*;
    #[cfg(feature = "client")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "server")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(feature = "client")]
    use tokio::io::{AsyncRead, AsyncReadExt};
    #[cfg(feature = "server")]
    use tokio::io::{AsyncWrite, AsyncWriteExt};

    /// The [`StatusResponsePacket`].
    ///
    /// This packet can be received only after a [`StatusRequestPacket`] and will not close the connection, allowing for a
    /// ping sequence to be exchanged afterward.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Status_Response)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct StatusResponsePacket {
        /// The JSON response body that contains all self-reported server metadata.
        pub body: String,
    }

    impl Packet for StatusResponsePacket {
        fn get_packet_id() -> VarInt {
            0x00
        }
    }

    #[cfg(feature = "server")]
    impl WritePacket for StatusResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.body).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for StatusResponsePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let body = buffer.read_string().await?;

            Ok(Self { body })
        }
    }

    /// This is the response to a specific [`PingPacket`] that can be used to measure the server ping.
    ///
    /// This packet will be sent after a corresponding [`PingPacket`] and will have the same payload as the request. This
    /// also consumes the connection, ending the Server List Ping sequence.
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PongPacket {
        /// The arbitrary payload that was sent from the client (to identify the corresponding response).
        pub payload: u64,
    }

    impl Packet for PongPacket {
        fn get_packet_id() -> VarInt {
            0x01
        }
    }

    #[cfg(feature = "server")]
    impl WritePacket for PongPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.payload).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for PongPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let payload = buffer.read_u64().await?;

            Ok(Self { payload })
        }
    }
}

pub mod serverbound {
    use super::*;
    use crate::ReadPacket;
    #[cfg(feature = "client")]
    use crate::WritePacket;
    #[cfg(feature = "server")]
    use tokio::io::AsyncRead;
    #[cfg(feature = "client")]
    use tokio::io::AsyncWrite;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The [`StatusRequestPacket`].
    ///
    /// The status can only be requested once immediately after the handshake, before any ping. The
    /// server won't respond otherwise.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Status_Request)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct StatusRequestPacket;

    impl Packet for StatusRequestPacket {
        fn get_packet_id() -> VarInt {
            0x00
        }
    }

    #[cfg(feature = "client")]
    impl WritePacket for StatusRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for StatusRequestPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The [`PingPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Ping_Request_(status))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PingPacket {
        /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
        pub payload: u64,
    }

    impl Packet for PingPacket {
        fn get_packet_id() -> VarInt {
            0x01
        }
    }

    #[cfg(feature = "client")]
    impl WritePacket for PingPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.payload).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for PingPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let payload = buffer.read_u64().await?;

            Ok(Self { payload })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_packet;

    #[tokio::test]
    async fn write_read_clientbound_status_response_packet() {
        assert_packet::<clientbound::StatusResponsePacket>(0x00).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_pong_packet() {
        assert_packet::<clientbound::PongPacket>(0x01).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_status_request_packet() {
        assert_packet::<serverbound::StatusRequestPacket>(0x00).await;
    }
}
