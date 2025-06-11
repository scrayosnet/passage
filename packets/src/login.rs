#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::Packet;
use crate::VarInt;
use uuid::Uuid;

pub mod clientbound {
    use super::{Error, Packet, Uuid, VarInt};
    use crate::VerifyToken;
    #[cfg(feature = "client")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "server")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(test)]
    use fake::Dummy;
    #[cfg(feature = "client")]
    use tokio::io::AsyncRead;
    #[cfg(feature = "server")]
    use tokio::io::AsyncWrite;

    /// The [`DisconnectPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Disconnect_(login))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct DisconnectPacket {
        /// The JSON text component containing the reason of the disconnect.
        pub reason: String,
    }

    impl Packet for DisconnectPacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "server")]
    impl WritePacket for DisconnectPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.reason).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for DisconnectPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let reason = buffer.read_string().await?;

            Ok(Self { reason })
        }
    }

    /// The [`EncryptionRequestPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Encryption_Request)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct EncryptionRequestPacket {
        // ignore max size
        pub server_id: String,
        pub public_key: Vec<u8>,
        pub verify_token: VerifyToken,
        pub should_authenticate: bool,
    }

    impl Packet for EncryptionRequestPacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "server")]
    impl WritePacket for EncryptionRequestPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.server_id).await?;
            buffer.write_bytes(&self.public_key).await?;
            buffer.write_bytes(&self.verify_token).await?;
            buffer.write_bool(self.should_authenticate).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for EncryptionRequestPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let server_id = buffer.read_string().await?;
            let public_key = buffer.read_bytes().await?;
            let verify_token = buffer
                .read_bytes()
                .await?
                .try_into()
                .map_err(|_| Error::ArrayConversionFailed)?;
            let should_authenticate = buffer.read_bool().await?;

            Ok(Self {
                server_id,
                public_key,
                verify_token,
                should_authenticate,
            })
        }
    }

    /// The [`LoginSuccessPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Success)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct LoginSuccessPacket {
        pub user_id: Uuid,
        pub user_name: String,
        // properties - we don't need those
    }

    impl Packet for LoginSuccessPacket {
        const ID: VarInt = 0x02;
    }

    #[cfg(feature = "server")]
    impl WritePacket for LoginSuccessPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_uuid(&self.user_id).await?;
            buffer.write_string(&self.user_name).await?;
            // no properties in array
            buffer.write_varint(0).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for LoginSuccessPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let user_id = buffer.read_uuid().await?;
            let user_name = buffer.read_string().await?;
            // expect no properties in array
            let _properties = buffer.read_varint().await?;

            Ok(Self { user_id, user_name })
        }
    }

    /// The [`SetCompressionPacket`]. (Placeholder)
    ///
    /// Enables compression. If compression is enabled, all following packets are encoded in the compressed
    /// packets format. Negative values will disable compression, meaning the packets format should remain
    /// in the uncompressed packets format. However, this packets is entirely optional, and if not sent,
    /// compression will also not be enabled (the vanilla server does not send the packets when compression
    /// is disabled).
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Set_Compression)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct SetCompressionPacket;

    impl Packet for SetCompressionPacket {
        const ID: VarInt = 0x03;
    }

    #[cfg(feature = "server")]
    impl WritePacket for SetCompressionPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for SetCompressionPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The [`LoginPluginRequestPacket`]. (Placeholder)
    ///
    /// Used to implement a custom handshake flow together with Login Plugin Response. Unlike plugin
    /// messages in "play" mode, these messages follow a lock-step request/response scheme, where the
    /// client is expected to respond to a request indicating whether it understood. The vanilla client
    /// always responds that it hasn't understood, and sends an empty payload.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Plugin_Request)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct LoginPluginRequestPacket;

    impl Packet for LoginPluginRequestPacket {
        const ID: VarInt = 0x04;
    }

    #[cfg(feature = "server")]
    impl WritePacket for LoginPluginRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for LoginPluginRequestPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The [`CookieRequestPacket`]. (Placeholder)
    ///
    /// Requests a cookie that was previously stored.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Request_(login))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct CookieRequestPacket {
        pub key: String,
    }

    impl Packet for CookieRequestPacket {
        const ID: VarInt = 0x05;
    }

    #[cfg(feature = "server")]
    impl WritePacket for CookieRequestPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for CookieRequestPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;

            Ok(Self { key })
        }
    }
}

pub mod serverbound {
    use super::{Error, Packet, VarInt};
    #[cfg(feature = "server")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "client")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(test)]
    use fake::Dummy;
    #[cfg(feature = "server")]
    use tokio::io::AsyncRead;
    #[cfg(feature = "client")]
    use tokio::io::AsyncWrite;
    use uuid::Uuid;

    /// The [`LoginStartPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Start)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct LoginStartPacket {
        pub user_name: String,
        pub user_id: Uuid,
    }

    impl Packet for LoginStartPacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "client")]
    impl WritePacket for LoginStartPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.user_name).await?;
            buffer.write_uuid(&self.user_id).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginStartPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let name = buffer.read_string().await?;
            let user_id = buffer.read_uuid().await?;

            Ok(Self {
                user_name: name,
                user_id,
            })
        }
    }

    /// The [`EncryptionResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Encryption_Response)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct EncryptionResponsePacket {
        pub shared_secret: Vec<u8>,
        pub verify_token: Vec<u8>,
    }

    impl Packet for EncryptionResponsePacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "client")]
    impl WritePacket for EncryptionResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_bytes(&self.shared_secret).await?;
            buffer.write_bytes(&self.verify_token).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for EncryptionResponsePacket {
        async fn read_from_buffer<S>(reader: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let shared_secret = reader.read_bytes().await?;
            let verify_token = reader.read_bytes().await?;

            Ok(Self {
                shared_secret,
                verify_token,
            })
        }
    }

    /// The [`LoginPluginResponsePacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Plugin_Response)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct LoginPluginResponsePacket;

    impl Packet for LoginPluginResponsePacket {
        const ID: VarInt = 0x02;
    }

    #[cfg(feature = "client")]
    impl WritePacket for LoginPluginResponsePacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginPluginResponsePacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The [`LoginAcknowledgedPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Acknowledged)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct LoginAcknowledgedPacket;

    impl Packet for LoginAcknowledgedPacket {
        const ID: VarInt = 0x03;
    }

    #[cfg(feature = "client")]
    impl WritePacket for LoginAcknowledgedPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginAcknowledgedPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The [`CookieResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Response_(login))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct CookieResponsePacket {
        pub key: String,
        pub payload: Option<Vec<u8>>,
    }

    impl Packet for CookieResponsePacket {
        const ID: VarInt = 0x04;
    }

    #[cfg(feature = "client")]
    impl WritePacket for CookieResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;
            buffer.write_bool(self.payload.is_some()).await?;
            if let Some(payload) = &self.payload {
                buffer.write_bytes(payload).await?;
            }

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for CookieResponsePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;
            let has_payload = buffer.read_bool().await?;
            let mut payload = None;
            if has_payload {
                payload = Some(buffer.read_bytes().await?);
            }

            Ok(Self { key, payload })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_packet;

    #[tokio::test]
    async fn write_read_clientbound_disconnect_packet() {
        assert_packet::<clientbound::DisconnectPacket>(0x00).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_encryption_request_packet() {
        assert_packet::<clientbound::EncryptionRequestPacket>(0x01).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_login_success_packet() {
        assert_packet::<clientbound::LoginSuccessPacket>(0x02).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_set_compression_packet() {
        assert_packet::<clientbound::SetCompressionPacket>(0x03).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_login_plugin_request_packet() {
        assert_packet::<clientbound::LoginPluginRequestPacket>(0x04).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_cookie_request_packet() {
        assert_packet::<clientbound::CookieRequestPacket>(0x05).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_login_start_packet() {
        assert_packet::<serverbound::LoginStartPacket>(0x00).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_encryption_response_packet() {
        assert_packet::<serverbound::EncryptionResponsePacket>(0x01).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_login_plugin_response_packet() {
        assert_packet::<serverbound::LoginPluginResponsePacket>(0x02).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_login_acknowledged_packet() {
        assert_packet::<serverbound::LoginAcknowledgedPacket>(0x03).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_cookie_response_packet() {
        assert_packet::<serverbound::CookieResponsePacket>(0x04).await;
    }
}
