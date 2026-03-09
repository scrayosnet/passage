#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::Packet;
use crate::VarInt;
use uuid::Uuid;

pub mod clientbound {
    use super::{Error, Packet, Uuid, VarInt};
    #[cfg(feature = "client")]
    use crate::io::reader::{Read, ReadPacket, ReadPacketExt};
    #[cfg(feature = "server")]
    use crate::io::writer::{Write, WritePacket, WritePacketExt};
    use crate::VerifyToken;
    #[cfg(test)]
    use fake::Dummy;
    use tracing::instrument;

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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.reason)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for DisconnectPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let reason = src.read_string()?;

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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.server_id)?;
            dst.write_bytes(&self.public_key)?;
            dst.write_bytes(&self.verify_token)?;
            dst.write_bool(self.should_authenticate)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for EncryptionRequestPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let server_id = src.read_string()?;
            let public_key = src.read_bytes()?;
            let verify_token = src
                .read_bytes()
                ?
                .try_into()
                .map_err(|_| Error::ArrayConversionFailed)?;
            let should_authenticate = src.read_bool()?;

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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_uuid(&self.user_id)?;
            dst.write_string(&self.user_name)?;
            // no properties in the array
            dst.write_varint(0)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for LoginSuccessPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let user_id = src.read_uuid()?;
            let user_name = src.read_string()?;
            // expect no properties in the array
            let _properties = src.read_varint()?;

            Ok(Self { user_id, user_name })
        }
    }

    /// The [`SetCompressionPacket`]. (Placeholder)
    ///
    /// Enables compression. If compression is enabled, all following packets are encoded in the compressed
    /// packets format. Negative values will disable compression, meaning the packets format should remain
    /// in the uncompressed packets format. However, this packet is entirely optional, and if not sent,
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
        fn write_packet(&self, _dst: &mut impl Write) -> Result<(), Error> {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for SetCompressionPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(_src: &mut impl Read) -> Result<Self, Error> {
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
        fn write_packet(&self, _dst: &mut impl Write) -> Result<(), Error> {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for LoginPluginRequestPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(_src: &mut impl Read) -> Result<Self, Error> {
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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.key)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for CookieRequestPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let key = src.read_string()?;

            Ok(Self { key })
        }
    }
}

pub mod serverbound {
    use super::{Error, Packet, Uuid, VarInt};
    #[cfg(feature = "server")]
    use crate::io::reader::{Read, ReadPacket, ReadPacketExt};
    #[cfg(feature = "client")]
    use crate::io::writer::{Write, WritePacket, WritePacketExt};
    #[cfg(test)]
    use fake::Dummy;
    use serde::Deserialize;
    use tracing::instrument;

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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.user_name)?;
            dst.write_uuid(&self.user_id)?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginStartPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let name = src.read_string()?;
            let user_id = src.read_uuid()?;

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
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_bytes(&self.shared_secret)?;
            dst.write_bytes(&self.verify_token)?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for EncryptionResponsePacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let shared_secret = src.read_bytes()?;
            let verify_token = src.read_bytes()?;

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
        fn write_packet(&self, _dst: &mut impl Write) -> Result<(), Error> {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginPluginResponsePacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(_src: &mut impl Read) -> Result<Self, Error> {
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
        fn write_packet(&self, _dst: &mut impl Write) -> Result<(), Error> {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for LoginAcknowledgedPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(_src: &mut impl Read) -> Result<Self, Error> {
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

    impl CookieResponsePacket {
        /// Decodes the payload into the provided type. Returns `None` if the payload is empty.
        pub fn decode<'a, T: Deserialize<'a>>(&'a self) -> Result<Option<T>, serde_json::Error> {
            let Some(payload) = &self.payload else {
                return Ok(None);
            };
            serde_json::from_slice(payload)
        }
    }

    impl Packet for CookieResponsePacket {
        const ID: VarInt = 0x04;
    }

    #[cfg(feature = "client")]
    impl WritePacket for CookieResponsePacket {
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.key)?;
            dst.write_bool(self.payload.is_some())?;
            if let Some(payload) = &self.payload {
                dst.write_bytes(payload)?;
            }

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for CookieResponsePacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let key = src.read_string()?;
            let has_payload = src.read_bool()?;
            let mut payload = None;
            if has_payload {
                payload = Some(src.read_bytes()?);
            }

            Ok(Self { key, payload })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::tests::assert_packet;

    #[test]
    fn write_read_clientbound_disconnect_packet() {
        assert_packet::<clientbound::DisconnectPacket>(0x00);
    }

    #[test]
    fn write_read_clientbound_encryption_request_packet() {
        assert_packet::<clientbound::EncryptionRequestPacket>(0x01);
    }

    #[test]
    fn write_read_clientbound_login_success_packet() {
        assert_packet::<clientbound::LoginSuccessPacket>(0x02);
    }

    #[test]
    fn write_read_clientbound_set_compression_packet() {
        assert_packet::<clientbound::SetCompressionPacket>(0x03);
    }

    #[test]
    fn write_read_clientbound_login_plugin_request_packet() {
        assert_packet::<clientbound::LoginPluginRequestPacket>(0x04);
    }

    #[test]
    fn write_read_clientbound_cookie_request_packet() {
        assert_packet::<clientbound::CookieRequestPacket>(0x05);
    }

    #[test]
    fn write_read_serverbound_login_start_packet() {
        assert_packet::<serverbound::LoginStartPacket>(0x00);
    }

    #[test]
    fn write_read_serverbound_encryption_response_packet() {
        assert_packet::<serverbound::EncryptionResponsePacket>(0x01);
    }

    #[test]
    fn write_read_serverbound_login_plugin_response_packet() {
        assert_packet::<serverbound::LoginPluginResponsePacket>(0x02);
    }

    #[test]
    fn write_read_serverbound_login_acknowledged_packet() {
        assert_packet::<serverbound::LoginAcknowledgedPacket>(0x03);
    }

    #[test]
    fn write_read_serverbound_cookie_response_packet() {
        assert_packet::<serverbound::CookieResponsePacket>(0x04);
    }
}
