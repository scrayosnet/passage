use crate::connection::{Connection, Phase, phase};
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet,
};
use crate::status::Protocol;
use fake::Dummy;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::debug;

pub mod outbound {
    use super::*;

    /// The outbound [`StatusResponsePacket`].
    ///
    /// This packet can be received only after a [`StatusRequestPacket`] and will not close the connection, allowing for a
    /// ping sequence to be exchanged afterward.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Status_Response)
    #[derive(Debug, Clone, Eq, PartialEq, Dummy)]
    pub struct StatusResponsePacket {
        /// The JSON response body that contains all self-reported server metadata.
        pub body: String,
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

    #[cfg(test)]
    impl InboundPacket for StatusResponsePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
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
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PongPacket {
        /// The arbitrary payload that was sent from the client (to identify the corresponding response).
        pub payload: u64,
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
    impl InboundPacket for PongPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let payload = buffer.read_u64().await?;

            Ok(Self { payload })
        }
    }
}

pub mod inbound {
    use super::*;

    /// The inbound [`StatusRequestPacket`].
    ///
    /// The status can only be requested once immediately after the handshake, before any ping. The
    /// server won't respond otherwise.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Status_Request)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct StatusRequestPacket;

    impl Packet for StatusRequestPacket {
        fn get_packet_id() -> usize {
            0x00
        }
    }

    #[cfg(test)]
    impl OutboundPacket for StatusRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for StatusRequestPacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received status request packet");
            phase!(
                con.phase,
                Phase::Status,
                client_address,
                server_address,
                server_port,
                protocol_version,
            );

            // get status from status supplier
            let status = con
                .status_supplier
                .get_status(
                    client_address,
                    (server_address, *server_port),
                    *protocol_version as Protocol,
                )
                .await?;

            // create a new status request packet and send it
            let json_response = serde_json::to_string(&status)?;

            // create a new status response packet and send it
            let request = outbound::StatusResponsePacket {
                body: json_response,
            };
            debug!(packet = debug(&request), "sending status response packet");
            con.write_packet(request).await?;

            Ok(())
        }
    }

    /// The inbound [`PingPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Ping_Request_(status))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PingPacket {
        /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
        pub payload: u64,
    }

    impl Packet for PingPacket {
        fn get_packet_id() -> usize {
            0x01
        }
    }

    #[cfg(test)]
    impl OutboundPacket for PingPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.payload).await?;

            Ok(())
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

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received ping packet");
            phase!(con.phase, Phase::Status,);

            // create a new pong packet and send it
            let pong_response = outbound::PongPacket::new(self.payload);
            debug!(packet = debug(&pong_response), "sending pong packet");
            con.write_packet(pong_response).await?;

            // close connection
            con.shutdown();

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
        assert_packet::<outbound::StatusResponsePacket>(0x00).await;
        assert_packet::<outbound::PongPacket>(0x01).await;

        assert_packet::<inbound::StatusRequestPacket>(0x00).await;
        assert_packet::<inbound::PingPacket>(0x01).await;
    }
}
