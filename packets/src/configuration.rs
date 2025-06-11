#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::Packet;
use crate::VarInt;
use crate::{ChatMode, DisplayedSkinParts, MainHand, ParticleStatus, ResourcePackResult};
use uuid::Uuid;

pub mod clientbound {
    use super::{Error, Packet, Uuid, VarInt};
    #[cfg(feature = "client")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "server")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(test)]
    use fake::Dummy;
    #[cfg(feature = "client")]
    use tokio::io::{AsyncRead, AsyncReadExt};
    #[cfg(feature = "server")]
    use tokio::io::{AsyncWrite, AsyncWriteExt};

    /// The clientbound [`CookieRequestPacket`]. (Placeholder)
    ///
    /// Requests a cookie that was previously stored.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Request_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct CookieRequestPacket;

    impl Packet for CookieRequestPacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "server")]
    impl WritePacket for CookieRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for CookieRequestPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`PluginMessagePacket`]. (Placeholder)
    ///
    /// Mods and plugins can use this to send their data. Minecraft itself uses several plugin channels.
    /// These internal channels are in the minecraft namespace. More information on how it works on
    /// Dinnerbone's blog. More documentation about internal and popular registered channels is here.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Clientbound_Plugin_Message_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PluginMessagePacket;

    impl Packet for PluginMessagePacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "server")]
    impl WritePacket for PluginMessagePacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for PluginMessagePacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`DisconnectPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Disconnect_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct DisconnectPacket {
        /// The text component containing the reason of the disconnect.
        pub reason: String,
    }

    impl Packet for DisconnectPacket {
        const ID: VarInt = 0x02;
    }

    #[cfg(feature = "server")]
    impl WritePacket for DisconnectPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_text_component(&self.reason).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for DisconnectPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let reason = buffer.read_text_component().await?;

            Ok(Self { reason })
        }
    }

    /// The clientbound [`FinishConfigurationPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Finish_Configuration)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct FinishConfigurationPacket;

    impl Packet for FinishConfigurationPacket {
        const ID: VarInt = 0x03;
    }

    #[cfg(feature = "server")]
    impl WritePacket for FinishConfigurationPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for FinishConfigurationPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`KeepAlivePacket`].
    ///
    /// The server will frequently send out a keep-alive, each containing a random ID. The client must
    /// respond with the same payload. If the client does not respond to a Keep Alive packet within 15
    /// seconds after it was sent, the server kicks the client. Vice versa, if the server does not send
    /// any keep-alive for 20 seconds, the client will disconnect and yield a "Timed out" exception.
    /// The vanilla server uses a system-dependent time in milliseconds to generate the keep alive ID value.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Finish_Configuration)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct KeepAlivePacket {
        pub id: u64,
    }

    impl KeepAlivePacket {
        #[must_use]
        pub const fn new(id: u64) -> Self {
            Self { id }
        }
    }

    impl Packet for KeepAlivePacket {
        const ID: VarInt = 0x04;
    }

    #[cfg(feature = "server")]
    impl WritePacket for KeepAlivePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.id).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for KeepAlivePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_u64().await?;

            Ok(Self { id })
        }
    }

    /// The clientbound [`PingPacket`]. (Placeholder)
    ///
    /// Packet is not used by the vanilla server. When sent to the client, the client responds with
    /// a Pong packet with the same id.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Ping_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PingPacket {
        pub id: i32,
    }

    impl Packet for PingPacket {
        const ID: VarInt = 0x05;
    }

    #[cfg(feature = "server")]
    impl WritePacket for PingPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_i32(self.id).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for PingPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_i32().await?;

            Ok(Self { id })
        }
    }

    /// The clientbound [`ResetChatPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Reset_Chat)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct ResetChatPacket;

    impl Packet for ResetChatPacket {
        const ID: VarInt = 0x06;
    }

    #[cfg(feature = "server")]
    impl WritePacket for ResetChatPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for ResetChatPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`RegistryDataPacket`]. (Placeholder)
    ///
    /// Represents certain registries that are sent from the server and are applied on the client.
    /// See Registry Data for details.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Registry_Data_2)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct RegistryDataPacket;

    impl Packet for RegistryDataPacket {
        const ID: VarInt = 0x07;
    }

    #[cfg(feature = "server")]
    impl WritePacket for RegistryDataPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for RegistryDataPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`RemoveResourcePackPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Remove_Resource_Pack_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct RemoveResourcePackPacket;

    impl Packet for RemoveResourcePackPacket {
        const ID: VarInt = 0x08;
    }

    #[cfg(feature = "server")]
    impl WritePacket for RemoveResourcePackPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for RemoveResourcePackPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`AddResourcePackPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Add_Resource_Pack_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct AddResourcePackPacket {
        pub uuid: Uuid,
        pub url: String,
        pub hash: String,
        pub forced: bool,
        /// The JSON response message.
        pub prompt_message: Option<String>,
    }

    impl Packet for AddResourcePackPacket {
        const ID: VarInt = 0x09;
    }

    #[cfg(feature = "server")]
    impl WritePacket for AddResourcePackPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_uuid(&self.uuid).await?;
            buffer.write_string(&self.url).await?;
            buffer.write_string(&self.hash).await?;
            buffer.write_bool(self.forced).await?;

            buffer.write_bool(self.prompt_message.is_some()).await?;
            if let Some(prompt_message) = &self.prompt_message {
                buffer.write_text_component(prompt_message).await?;
            }

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for AddResourcePackPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let uuid = buffer.read_uuid().await?;
            let url = buffer.read_string().await?;
            let hash = buffer.read_string().await?;
            let forced = buffer.read_bool().await?;

            let mut prompt_message = None;
            if buffer.read_bool().await? {
                prompt_message = Some(buffer.read_text_component().await?);
            }

            Ok(Self {
                uuid,
                url,
                hash,
                forced,
                prompt_message,
            })
        }
    }

    /// The clientbound [`StoreCookiePacket`]. (Placeholder)
    ///
    /// Stores some arbitrary data on the client, which persists between server transfers. The vanilla
    /// client only accepts cookies of up to 5 kiB in size.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Store_Cookie_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct StoreCookiePacket {
        pub key: String,
        pub payload: Vec<u8>,
    }

    impl Packet for StoreCookiePacket {
        const ID: VarInt = 0x0A;
    }

    #[cfg(feature = "server")]
    impl WritePacket for StoreCookiePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;
            buffer.write_bytes(&self.payload).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for StoreCookiePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;
            let payload = buffer.read_bytes().await?;

            Ok(Self { key, payload })
        }
    }

    /// The clientbound [`TransferPacket`].
    ///
    /// Notifies the client that it should transfer to the given server. Cookies previously stored are
    /// preserved between server transfers.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct TransferPacket {
        pub host: String,
        pub port: u16,
    }

    impl Packet for TransferPacket {
        const ID: VarInt = 0x0B;
    }

    #[cfg(feature = "server")]
    impl WritePacket for TransferPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.host).await?;
            buffer.write_varint(VarInt::from(self.port)).await?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for TransferPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let host = buffer.read_string().await?;
            let port = buffer.read_varint().await? as u16;

            Ok(Self { host, port })
        }
    }

    /// The clientbound [`FeatureFlagsPacket`]. (Placeholder)
    ///
    /// Used to enable and disable features, generally experimental ones, on the client.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Feature_Flags)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct FeatureFlagsPacket;

    impl Packet for FeatureFlagsPacket {
        const ID: VarInt = 0x0C;
    }

    #[cfg(feature = "server")]
    impl WritePacket for FeatureFlagsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for FeatureFlagsPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`UpdateTagsPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Update_Tags_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct UpdateTagsPacket;

    impl Packet for UpdateTagsPacket {
        const ID: VarInt = 0x0D;
    }

    #[cfg(feature = "server")]
    impl WritePacket for UpdateTagsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for UpdateTagsPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`KnownPacksPacket`]. (Placeholder)
    ///
    /// Informs the client of which data packs are present on the server. The client is expected to respond
    /// with its own Serverbound Known Packs packets. The vanilla server does not continue with Configuration
    /// until it receives a response. The vanilla client requires the minecraft:core pack with version
    /// 1.21.4 for a normal login sequence. This packet must be sent before the Registry Data packets.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Clientbound_Known_Packs)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct KnownPacksPacket;

    impl Packet for KnownPacksPacket {
        const ID: VarInt = 0x0E;
    }

    #[cfg(feature = "server")]
    impl WritePacket for KnownPacksPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for KnownPacksPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`CustomReportDetailsPacket`]. (Placeholder)
    ///
    /// Contains a list of key-value text entries that are included in any crash or disconnection report
    /// generated during connection to the server.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Custom_Report_Details_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct CustomReportDetailsPacket;

    impl Packet for CustomReportDetailsPacket {
        const ID: VarInt = 0x0F;
    }

    #[cfg(feature = "server")]
    impl WritePacket for CustomReportDetailsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for CustomReportDetailsPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The clientbound [`ServerLinksPacket`]. (Placeholder)
    ///
    /// This packet contains a list of links that the vanilla client will display in the menu available
    /// from the pause menu. Link labels can be built-in or custom (i.e., any text).
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Server_Links_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct ServerLinksPacket;

    impl Packet for ServerLinksPacket {
        const ID: VarInt = 0x10;
    }

    #[cfg(feature = "server")]
    impl WritePacket for ServerLinksPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for ServerLinksPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }
}

pub mod serverbound {
    use super::{
        ChatMode, DisplayedSkinParts, Error, MainHand, Packet, ParticleStatus, ResourcePackResult,
        Uuid, VarInt,
    };
    #[cfg(feature = "server")]
    use crate::{AsyncReadPacket, ReadPacket};
    #[cfg(feature = "client")]
    use crate::{AsyncWritePacket, WritePacket};
    #[cfg(test)]
    use fake::Dummy;
    #[cfg(feature = "server")]
    use tokio::io::{AsyncRead, AsyncReadExt};
    #[cfg(feature = "client")]
    use tokio::io::{AsyncWrite, AsyncWriteExt};

    /// The serverbound [`ClientInformationPacket`]. (Placeholder)
    ///
    /// Sent when the player connects or when settings are changed.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Client_Information_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct ClientInformationPacket {
        pub locale: String,
        pub view_distance: i8,
        pub chat_mode: ChatMode,
        pub chat_colors: bool,
        pub displayed_skin_parts: DisplayedSkinParts,
        pub main_hand: MainHand,
        pub enable_text_filtering: bool,
        pub allow_server_listing: bool,
        pub particle_status: ParticleStatus,
    }

    impl Packet for ClientInformationPacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "client")]
    impl WritePacket for ClientInformationPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.locale).await?;
            buffer.write_i8(self.view_distance).await?;
            buffer.write_varint(self.chat_mode.into()).await?;
            buffer.write_bool(self.chat_colors).await?;
            buffer.write_u8(self.displayed_skin_parts.0).await?;
            buffer.write_varint(self.main_hand.into()).await?;
            buffer.write_bool(self.enable_text_filtering).await?;
            buffer.write_bool(self.allow_server_listing).await?;
            buffer.write_varint(self.particle_status.into()).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for ClientInformationPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let locale = buffer.read_string().await?;
            let view_distance = buffer.read_i8().await?;
            let chat_mode = buffer.read_varint().await?.try_into()?;
            let chat_colors = buffer.read_bool().await?;
            let displayed_skin_parts = DisplayedSkinParts(buffer.read_u8().await?);
            let main_hand = buffer.read_varint().await?.try_into()?;
            let enable_text_filtering = buffer.read_bool().await?;
            let allow_server_listing = buffer.read_bool().await?;
            let particle_status = buffer.read_varint().await?.try_into()?;

            Ok(Self {
                locale,
                view_distance,
                chat_mode,
                chat_colors,
                displayed_skin_parts,
                main_hand,
                enable_text_filtering,
                allow_server_listing,
                particle_status,
            })
        }
    }

    /// The serverbound [`CookieResponsePacket`]. (Placeholder)
    ///
    /// Response to a Cookie Request (configuration) from the server. The vanilla server only accepts
    /// responses of up to 5 kiB in size.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Response_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct CookieResponsePacket;

    impl Packet for CookieResponsePacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "client")]
    impl WritePacket for CookieResponsePacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for CookieResponsePacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The serverbound [`PluginMessagePacket`]. (Placeholder)
    ///
    /// Mods and plugins can use this to send their data. Minecraft itself uses some plugin channels.
    /// These internal channels are in the minecraft namespace. More documentation on this:
    /// <https://dinnerbone.com/blog/2012/01/13/minecraft-plugin-channels-messaging/>
    ///
    /// Note that the length of Data is known only from the packet length, since the packet has no length
    /// field of any kind.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Serverbound_Plugin_Message_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PluginMessagePacket;

    impl Packet for PluginMessagePacket {
        const ID: VarInt = 0x02;
    }

    #[cfg(feature = "client")]
    impl WritePacket for PluginMessagePacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for PluginMessagePacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The serverbound [`AckFinishConfigurationPacket`]. (Placeholder)
    ///
    /// Sent by the client to notify the server that the configuration process has finished. It is sent
    /// in response to the server's Finish Configuration.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Acknowledge_Finish_Configuration)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct AckFinishConfigurationPacket;

    impl Packet for AckFinishConfigurationPacket {
        const ID: VarInt = 0x03;
    }

    #[cfg(feature = "client")]
    impl WritePacket for AckFinishConfigurationPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for AckFinishConfigurationPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The serverbound [`KeepAlivePacket`].
    ///
    /// The server will frequently send out a keep-alive, each containing a random ID. The client must
    /// respond with the same payload. If the client does not respond to a Keep Alive packet within 15
    /// seconds after it was sent, the server kicks the client. Vice versa, if the server does not send
    /// any keep-alive for 20 seconds, the client will disconnect and yield a "Timed out" exception.
    /// The vanilla server uses a system-dependent time in milliseconds to generate the keep alive ID value.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Finish_Configuration)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct KeepAlivePacket {
        pub id: u64,
    }

    impl Packet for KeepAlivePacket {
        const ID: VarInt = 0x04;
    }

    #[cfg(feature = "client")]
    impl WritePacket for KeepAlivePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.id).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for KeepAlivePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_u64().await?;
            Ok(Self { id })
        }
    }

    /// The serverbound [`PongPacket`]. (Placeholder)
    ///
    /// Response to the clientbound packets (Ping) with the same id
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Pong_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PongPacket {
        pub id: i32,
    }

    impl Packet for PongPacket {
        const ID: VarInt = 0x05;
    }

    #[cfg(feature = "client")]
    impl WritePacket for PongPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_i32(self.id).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for PongPacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_i32().await?;

            Ok(Self { id })
        }
    }

    /// The serverbound [`ResourcePackResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Resource_Pack_Response_(configuration))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct ResourcePackResponsePacket {
        pub uuid: Uuid,
        pub result: ResourcePackResult,
    }

    impl Packet for ResourcePackResponsePacket {
        const ID: VarInt = 0x06;
    }

    #[cfg(feature = "client")]
    impl WritePacket for ResourcePackResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_uuid(&self.uuid).await?;
            buffer.write_varint(self.result.into()).await?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for ResourcePackResponsePacket {
        async fn read_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let uuid = buffer.read_uuid().await?;
            let result = buffer.read_varint().await?.try_into()?;

            Ok(Self { uuid, result })
        }
    }

    /// The serverbound [`KnownPacksPacket`]. (Placeholder)
    ///
    /// Informs the server of which data packs are present on the client. The client sends this in response
    /// to Clientbound Known Packs. If the client specifies a pack in this packet, the server should omit
    /// its contained data from the Registry Data packets.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Serverbound_Known_Packs)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct KnownPacksPacket;

    impl Packet for KnownPacksPacket {
        const ID: VarInt = 0x07;
    }

    #[cfg(feature = "client")]
    impl WritePacket for KnownPacksPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for KnownPacksPacket {
        async fn read_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::assert_packet;

    #[tokio::test]
    async fn write_read_clientbound_cookie_request_packet() {
        assert_packet::<clientbound::CookieRequestPacket>(0x00).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_plugin_message_packet() {
        assert_packet::<clientbound::PluginMessagePacket>(0x01).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_disconnect_packet() {
        assert_packet::<clientbound::DisconnectPacket>(0x02).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_finish_configuration_packet() {
        assert_packet::<clientbound::FinishConfigurationPacket>(0x03).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_keep_alive_packet() {
        assert_packet::<clientbound::KeepAlivePacket>(0x04).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_ping_packet() {
        assert_packet::<clientbound::PingPacket>(0x05).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_reset_chat_packet() {
        assert_packet::<clientbound::ResetChatPacket>(0x06).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_registry_data_packet() {
        assert_packet::<clientbound::RegistryDataPacket>(0x07).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_remove_resource_pack_packet() {
        assert_packet::<clientbound::RemoveResourcePackPacket>(0x08).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_add_resource_pack_packet() {
        assert_packet::<clientbound::AddResourcePackPacket>(0x09).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_store_cookie_packet() {
        assert_packet::<clientbound::StoreCookiePacket>(0x0A).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_transfer_packet() {
        assert_packet::<clientbound::TransferPacket>(0x0B).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_feature_flags_packet() {
        assert_packet::<clientbound::FeatureFlagsPacket>(0x0C).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_update_tags_packet() {
        assert_packet::<clientbound::UpdateTagsPacket>(0x0D).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_known_packs_packet() {
        assert_packet::<clientbound::KnownPacksPacket>(0x0E).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_custom_report_details_packet() {
        assert_packet::<clientbound::CustomReportDetailsPacket>(0x0F).await;
    }

    #[tokio::test]
    async fn write_read_clientbound_server_links_packet() {
        assert_packet::<clientbound::ServerLinksPacket>(0x10).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_client_information_packet() {
        assert_packet::<serverbound::ClientInformationPacket>(0x00).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_cookie_response_packet() {
        assert_packet::<serverbound::CookieResponsePacket>(0x01).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_plugin_message_packet() {
        assert_packet::<serverbound::PluginMessagePacket>(0x02).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_ack_finish_configuration_packet() {
        assert_packet::<serverbound::AckFinishConfigurationPacket>(0x03).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_keep_alive_packet() {
        assert_packet::<serverbound::KeepAlivePacket>(0x04).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_pong_packet() {
        assert_packet::<serverbound::PongPacket>(0x05).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_resource_pack_response_packet() {
        assert_packet::<serverbound::ResourcePackResponsePacket>(0x06).await;
    }

    #[tokio::test]
    async fn write_read_serverbound_known_packs_packet() {
        assert_packet::<serverbound::KnownPacksPacket>(0x07).await;
    }
}
