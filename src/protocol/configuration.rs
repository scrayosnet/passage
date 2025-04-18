use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, ChatMode, DisplayedSkinParts, Error, InboundPacket,
    MainHand, OutboundPacket, Packet, ParticleStatus, ResourcePackResult,
};
use fake::Dummy;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub mod outbound {
    use super::*;
    use crate::protocol::VarInt;

    /// The outbound [`CookieRequestPacket`]. (Placeholder)
    ///
    /// Requests a cookie that was previously stored.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Request_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct CookieRequestPacket;

    impl Packet for CookieRequestPacket {
        fn get_packet_id() -> usize {
            0x00
        }
    }

    impl OutboundPacket for CookieRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for CookieRequestPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`PluginMessagePacket`]. (Placeholder)
    ///
    /// Mods and plugins can use this to send their data. Minecraft itself uses several plugin channels.
    /// These internal channels are in the minecraft namespace. More information on how it works on
    /// Dinnerbone's blog. More documentation about internal and popular registered channels are here.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Clientbound_Plugin_Message_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PluginMessagePacket;

    impl Packet for PluginMessagePacket {
        fn get_packet_id() -> usize {
            0x01
        }
    }

    impl OutboundPacket for PluginMessagePacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for PluginMessagePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`DisconnectPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Disconnect_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct DisconnectPacket {
        /// The text component containing the reason of the disconnect.
        pub(crate) reason: String,
    }

    impl Packet for DisconnectPacket {
        fn get_packet_id() -> usize {
            0x02
        }
    }

    impl OutboundPacket for DisconnectPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.reason).await?;
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for DisconnectPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let reason = buffer.read_string().await?;

            Ok(Self { reason })
        }
    }

    /// The outbound [`FinishConfigurationPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Finish_Configuration)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct FinishConfigurationPacket;

    impl Packet for FinishConfigurationPacket {
        fn get_packet_id() -> usize {
            0x03
        }
    }

    impl OutboundPacket for FinishConfigurationPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for FinishConfigurationPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`KeepAlivePacket`].
    ///
    /// The server will frequently send out a keep-alive, each containing a random ID. The client must
    /// respond with the same payload. If the client does not respond to a Keep Alive packet within 15
    /// seconds after it was sent, the server kicks the client. Vice versa, if the server does not send
    /// any keep-alives for 20 seconds, the client will disconnect and yields a "Timed out" exception.
    /// The vanilla server uses a system-dependent time in milliseconds to generate the keep alive ID value.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Finish_Configuration)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct KeepAlivePacket {
        id: u64,
    }

    impl KeepAlivePacket {
        pub const fn new(id: u64) -> Self {
            Self { id }
        }
    }

    impl Packet for KeepAlivePacket {
        fn get_packet_id() -> usize {
            0x04
        }
    }

    impl OutboundPacket for KeepAlivePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.id).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for KeepAlivePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_u64().await?;

            Ok(Self { id })
        }
    }

    /// The outbound [`PingPacket`]. (Placeholder)
    ///
    /// Packet is not used by the vanilla server. When sent to the client, client responds with a Pong
    /// packet with the same id.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Ping_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PingPacket {
        pub id: i32,
    }

    impl Packet for PingPacket {
        fn get_packet_id() -> usize {
            0x05
        }
    }

    impl OutboundPacket for PingPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_i32(self.id).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for PingPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_i32().await?;

            Ok(Self { id })
        }
    }

    /// The outbound [`ResetChatPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Reset_Chat)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct ResetChatPacket;

    impl Packet for ResetChatPacket {
        fn get_packet_id() -> usize {
            0x06
        }
    }

    impl OutboundPacket for ResetChatPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for ResetChatPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`RegistryDataPacket`]. (Placeholder)
    ///
    /// Represents certain registries that are sent from the server and are applied on the client.
    /// See Registry Data for details.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Registry_Data_2)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct RegistryDataPacket;

    impl Packet for RegistryDataPacket {
        fn get_packet_id() -> usize {
            0x07
        }
    }

    impl OutboundPacket for RegistryDataPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for RegistryDataPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`RemoveResourcePackPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Remove_Resource_Pack_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct RemoveResourcePackPacket;

    impl Packet for RemoveResourcePackPacket {
        fn get_packet_id() -> usize {
            0x08
        }
    }

    impl OutboundPacket for RemoveResourcePackPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for RemoveResourcePackPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`AddResourcePackPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Add_Resource_Pack_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct AddResourcePackPacket {
        pub uuid: Uuid,
        pub url: String,
        pub hash: String,
        pub forced: bool,
        /// The JSON response message.
        pub prompt_message: Option<String>,
    }

    impl Packet for AddResourcePackPacket {
        fn get_packet_id() -> usize {
            0x09
        }
    }

    impl OutboundPacket for AddResourcePackPacket {
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

    #[cfg(test)]
    impl InboundPacket for AddResourcePackPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
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

    /// The outbound [`StoreCookiePacket`]. (Placeholder)
    ///
    /// Stores some arbitrary data on the client, which persists between server transfers. The vanilla
    /// client only accepts cookies of up to 5 kiB in size.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Store_Cookie_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct StoreCookiePacket {
        pub key: String,
        pub payload: Vec<u8>,
    }

    impl Packet for StoreCookiePacket {
        fn get_packet_id() -> usize {
            0x0A
        }
    }

    impl OutboundPacket for StoreCookiePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;
            buffer.write_bytes(&self.payload).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for StoreCookiePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;
            let payload = buffer.read_bytes().await?;

            Ok(Self { key, payload })
        }
    }

    /// The outbound [`TransferPacket`].
    ///
    /// Notifies the client that it should transfer to the given server. Cookies previously stored are
    /// preserved between server transfers.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Transfer_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct TransferPacket {
        pub host: String,
        pub port: u16,
    }

    impl Packet for TransferPacket {
        fn get_packet_id() -> usize {
            0x0B
        }
    }

    impl OutboundPacket for TransferPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.host).await?;
            buffer.write_varint(self.port as VarInt).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for TransferPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let host = buffer.read_string().await?;
            let port = buffer.read_varint().await? as u16;

            Ok(Self { host, port })
        }
    }

    /// The outbound [`FeatureFlagsPacket`]. (Placeholder)
    ///
    /// Used to enable and disable features, generally experimental ones, on the client.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Feature_Flags)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct FeatureFlagsPacket;

    impl Packet for FeatureFlagsPacket {
        fn get_packet_id() -> usize {
            0x0C
        }
    }

    impl OutboundPacket for FeatureFlagsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for FeatureFlagsPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`UpdateTagsPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Update_Tags_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct UpdateTagsPacket;

    impl Packet for UpdateTagsPacket {
        fn get_packet_id() -> usize {
            0x0D
        }
    }

    impl OutboundPacket for UpdateTagsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for UpdateTagsPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`KnownPacksPacket`]. (Placeholder)
    ///
    /// Informs the client of which data packs are present on the server. The client is expected to respond
    /// with its own Serverbound Known Packs packet. The vanilla server does not continue with Configuration
    /// until it receives a response. The vanilla client requires the minecraft:core pack with version
    /// 1.21.4 for a normal login sequence. This packet must be sent before the Registry Data packets.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Clientbound_Known_Packs)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct KnownPacksPacket;

    impl Packet for KnownPacksPacket {
        fn get_packet_id() -> usize {
            0x0E
        }
    }

    impl OutboundPacket for KnownPacksPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for KnownPacksPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`CustomReportDetailsPacket`]. (Placeholder)
    ///
    /// Contains a list of key-value text entries that are included in any crash or disconnection report
    /// generated during connection to the server.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Custom_Report_Details_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct CustomReportDetailsPacket;

    impl Packet for CustomReportDetailsPacket {
        fn get_packet_id() -> usize {
            0x0F
        }
    }

    impl OutboundPacket for CustomReportDetailsPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for CustomReportDetailsPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`ServerLinksPacket`]. (Placeholder)
    ///
    /// This packet contains a list of links that the vanilla client will display in the menu available
    /// from the pause menu. Link labels can be built-in or custom (i.e., any text).
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Server_Links_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct ServerLinksPacket;

    impl Packet for ServerLinksPacket {
        fn get_packet_id() -> usize {
            0x10
        }
    }

    impl OutboundPacket for ServerLinksPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for ServerLinksPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }
}

pub mod inbound {
    use super::*;

    /// The inbound [`ClientInformationPacket`]. (Placeholder)
    ///
    /// Sent when the player connects, or when settings are changed.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Client_Information_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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
        fn get_packet_id() -> usize {
            0x00
        }
    }

    #[cfg(test)]
    impl OutboundPacket for ClientInformationPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.locale).await?;
            buffer.write_i8(self.view_distance).await?;
            buffer.write_varint(self.chat_mode.clone().into()).await?;
            buffer.write_bool(self.chat_colors).await?;
            buffer.write_u8(self.displayed_skin_parts.0).await?;
            buffer.write_varint(self.main_hand.clone().into()).await?;
            buffer.write_bool(self.enable_text_filtering).await?;
            buffer.write_bool(self.allow_server_listing).await?;
            buffer
                .write_varint(self.particle_status.clone().into())
                .await?;

            Ok(())
        }
    }

    impl InboundPacket for ClientInformationPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
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

    /// The inbound [`CookieResponsePacket`]. (Placeholder)
    ///
    /// Response to a Cookie Request (configuration) from the server. The vanilla server only accepts
    /// responses of up to 5 kiB in size.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Response_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct CookieResponsePacket;

    impl Packet for CookieResponsePacket {
        fn get_packet_id() -> usize {
            0x01
        }
    }

    #[cfg(test)]
    impl OutboundPacket for CookieResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for CookieResponsePacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The inbound [`PluginMessagePacket`]. (Placeholder)
    ///
    /// Mods and plugins can use this to send their data. Minecraft itself uses some plugin channels.
    /// These internal channels are in the minecraft namespace. More documentation on this:
    /// https://dinnerbone.com/blog/2012/01/13/minecraft-plugin-channels-messaging/
    ///
    /// Note that the length of Data is known only from the packet length, since the packet has no length
    /// field of any kind.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Serverbound_Plugin_Message_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PluginMessagePacket;

    impl Packet for PluginMessagePacket {
        fn get_packet_id() -> usize {
            0x02
        }
    }

    #[cfg(test)]
    impl OutboundPacket for PluginMessagePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for PluginMessagePacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The inbound [`AckFinishConfigurationPacket`]. (Placeholder)
    ///
    /// Sent by the client to notify the server that the configuration process has finished. It is sent
    /// in response to the server's Finish Configuration.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Acknowledge_Finish_Configuration)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct AckFinishConfigurationPacket;

    impl Packet for AckFinishConfigurationPacket {
        fn get_packet_id() -> usize {
            0x03
        }
    }

    #[cfg(test)]
    impl OutboundPacket for AckFinishConfigurationPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for AckFinishConfigurationPacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The inbound [`KeepAlivePacket`].
    ///
    /// The server will frequently send out a keep-alive, each containing a random ID. The client must
    /// respond with the same payload. If the client does not respond to a Keep Alive packet within 15
    /// seconds after it was sent, the server kicks the client. Vice versa, if the server does not send
    /// any keep-alives for 20 seconds, the client will disconnect and yields a "Timed out" exception.
    /// The vanilla server uses a system-dependent time in milliseconds to generate the keep alive ID value.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Finish_Configuration)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct KeepAlivePacket {
        pub id: u64,
    }

    impl Packet for KeepAlivePacket {
        fn get_packet_id() -> usize {
            0x04
        }
    }

    #[cfg(test)]
    impl OutboundPacket for KeepAlivePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_u64(self.id).await?;

            Ok(())
        }
    }

    impl InboundPacket for KeepAlivePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_u64().await?;
            Ok(Self { id })
        }
    }

    /// The inbound [`PongPacket`]. (Placeholder)
    ///
    /// Response to the clientbound packet (Ping) with the same id
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Pong_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct PongPacket {
        pub id: i32,
    }

    impl Packet for PongPacket {
        fn get_packet_id() -> usize {
            0x05
        }
    }

    #[cfg(test)]
    impl OutboundPacket for PongPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_i32(self.id).await?;

            Ok(())
        }
    }

    impl InboundPacket for PongPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let id = buffer.read_i32().await?;

            Ok(Self { id })
        }
    }

    /// The inbound [`ResourcePackResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Resource_Pack_Response_(configuration))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct ResourcePackResponsePacket {
        pub uuid: Uuid,
        pub result: ResourcePackResult,
    }

    impl Packet for ResourcePackResponsePacket {
        fn get_packet_id() -> usize {
            0x06
        }
    }

    #[cfg(test)]
    impl OutboundPacket for ResourcePackResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_uuid(&self.uuid).await?;
            buffer.write_varint(self.result.clone().into()).await?;

            Ok(())
        }
    }

    impl InboundPacket for ResourcePackResponsePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let uuid = buffer.read_uuid().await?;
            let result = buffer.read_varint().await?.try_into()?;

            Ok(Self { uuid, result })
        }
    }

    /// The inbound [`KnownPacksPacket`]. (Placeholder)
    ///
    /// Informs the server of which data packs are present on the client. The client sends this in response
    /// to Clientbound Known Packs. If the client specifies a pack in this packet, the server should omit
    /// its contained data from the Registry Data packet.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Serverbound_Known_Packs)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct KnownPacksPacket;

    impl Packet for KnownPacksPacket {
        fn get_packet_id() -> usize {
            0x07
        }
    }

    #[cfg(test)]
    impl OutboundPacket for KnownPacksPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for KnownPacksPacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
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
    use crate::protocol::tests::assert_packet;

    #[tokio::test]
    async fn packets() {
        assert_packet::<outbound::CookieRequestPacket>(0x00).await;
        assert_packet::<outbound::PluginMessagePacket>(0x01).await;
        assert_packet::<outbound::DisconnectPacket>(0x02).await;
        assert_packet::<outbound::FinishConfigurationPacket>(0x03).await;
        assert_packet::<outbound::KeepAlivePacket>(0x04).await;
        assert_packet::<outbound::PingPacket>(0x05).await;
        assert_packet::<outbound::ResetChatPacket>(0x06).await;
        assert_packet::<outbound::RegistryDataPacket>(0x07).await;
        assert_packet::<outbound::RemoveResourcePackPacket>(0x08).await;
        assert_packet::<outbound::AddResourcePackPacket>(0x09).await;
        assert_packet::<outbound::StoreCookiePacket>(0x0A).await;
        assert_packet::<outbound::TransferPacket>(0x0B).await;
        assert_packet::<outbound::FeatureFlagsPacket>(0x0C).await;
        assert_packet::<outbound::UpdateTagsPacket>(0x0D).await;
        assert_packet::<outbound::KnownPacksPacket>(0x0E).await;
        assert_packet::<outbound::CustomReportDetailsPacket>(0x0F).await;
        assert_packet::<outbound::ServerLinksPacket>(0x10).await;

        assert_packet::<inbound::ClientInformationPacket>(0x00).await;
        assert_packet::<inbound::CookieResponsePacket>(0x01).await;
        assert_packet::<inbound::PluginMessagePacket>(0x02).await;
        assert_packet::<inbound::AckFinishConfigurationPacket>(0x03).await;
        assert_packet::<inbound::KeepAlivePacket>(0x04).await;
        assert_packet::<inbound::PongPacket>(0x05).await;
        assert_packet::<inbound::ResourcePackResponsePacket>(0x06).await;
        assert_packet::<inbound::KnownPacksPacket>(0x07).await;
    }
}
