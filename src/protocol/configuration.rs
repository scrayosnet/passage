use crate::connection::Connection;
use crate::protocol::Error::Generic;
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet,
    ResourcePackResult,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

/// The outbound [`CookieRequestPacket`]. (Placeholder)
///
/// Requests a cookie that was previously stored.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Request_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`OutPluginMessagePacket`]. (Placeholder)
///
/// Mods and plugins can use this to send their data. Minecraft itself uses several plugin channels.
/// These internal channels are in the minecraft namespace. More information on how it works on
/// Dinnerbone's blog. More documentation about internal and popular registered channels are here.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Clientbound_Plugin_Message_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct OutPluginMessagePacket;

impl Packet for OutPluginMessagePacket {
    fn get_packet_id() -> usize {
        0x01
    }
}

impl OutboundPacket for OutPluginMessagePacket {
    async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The outbound [`DisconnectPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Disconnect_(configuration))
#[derive(Debug)]
pub struct DisconnectPacket {
    /// The JSON response reason that contains all self-reported server metadata.
    reason: String,
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

/// The outbound [`FinishConfigurationPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Finish_Configuration)
#[derive(Debug)]
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

/// The in/outbound [`KeepAlivePacket`].
///
/// The server will frequently send out a keep-alive, each containing a random ID. The client must
/// respond with the same payload. If the client does not respond to a Keep Alive packet within 15
/// seconds after it was sent, the server kicks the client. Vice versa, if the server does not send
/// any keep-alives for 20 seconds, the client will disconnect and yields a "Timed out" exception.
/// The vanilla server uses a system-dependent time in milliseconds to generate the keep alive ID value.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Finish_Configuration)
#[derive(Debug)]
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

impl InboundPacket for KeepAlivePacket {
    async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        let id = buffer.read_u64().await?;
        Ok(Self { id })
    }

    async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        if !con.last_keep_alive.replace(self.id, 0) {
            return Err(Generic("keep alive packet already received".to_string()));
        }

        Ok(())
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

/// The outbound [`PingPacket`]. (Placeholder)
///
/// Packet is not used by the vanilla server. When sent to the client, client responds with a Pong
/// packet with the same id.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Ping_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct PingPacket;

impl Packet for PingPacket {
    fn get_packet_id() -> usize {
        0x05
    }
}

impl OutboundPacket for PingPacket {
    async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The outbound [`ResetChatPacket`]. (Placeholder)
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Reset_Chat)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`RegistryDataPacket`]. (Placeholder)
///
/// Represents certain registries that are sent from the server and are applied on the client.
/// See Registry Data for details.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Registry_Data_2)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`RemoveResourcePackPacket`]. (Placeholder)
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Remove_Resource_Pack_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`AddResourcePackPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Add_Resource_Pack_(configuration))
#[derive(Debug)]
pub struct AddResourcePackPacket {
    pub uuid: Uuid,
    pub url: String,
    pub hash: String,
    pub forced: bool,
    /// The JSON response message.
    pub prompt_message: String,
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
        buffer.write_bool(true).await?;
        buffer.write_text_component(&self.prompt_message).await?;

        Ok(())
    }
}

/// The outbound [`StoreCookiePacket`]. (Placeholder)
///
/// Stores some arbitrary data on the client, which persists between server transfers. The vanilla
/// client only accepts cookies of up to 5 kiB in size.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Store_Cookie_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct StoreCookiePacket;

impl Packet for StoreCookiePacket {
    fn get_packet_id() -> usize {
        0x0A
    }
}

impl OutboundPacket for StoreCookiePacket {
    async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The outbound [`TransferPacket`].
///
/// Notifies the client that it should transfer to the given server. Cookies previously stored are
/// preserved between server transfers.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Transfer_(configuration))
#[derive(Debug)]
pub struct TransferPacket {
    pub host: String,
    pub port: usize,
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
        buffer.write_varint(self.port).await?;

        Ok(())
    }
}

/// The outbound [`FeatureFlagsPacket`]. (Placeholder)
///
/// Used to enable and disable features, generally experimental ones, on the client.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Feature_Flags)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`UpdateTagsPacket`]. (Placeholder)
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Update_Tags_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`OutKnownPacksPacket`]. (Placeholder)
///
/// Informs the client of which data packs are present on the server. The client is expected to respond
/// with its own Serverbound Known Packs packet. The vanilla server does not continue with Configuration
/// until it receives a response. The vanilla client requires the minecraft:core pack with version
/// 1.21.4 for a normal login sequence. This packet must be sent before the Registry Data packets.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Clientbound_Known_Packs)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct OutKnownPacksPacket;

impl Packet for OutKnownPacksPacket {
    fn get_packet_id() -> usize {
        0x0E
    }
}

impl OutboundPacket for OutKnownPacksPacket {
    async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The outbound [`CustomReportDetailsPacket`]. (Placeholder)
///
/// Contains a list of key-value text entries that are included in any crash or disconnection report
/// generated during connection to the server.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Custom_Report_Details_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`ServerLinksPacket`]. (Placeholder)
///
/// This packet contains a list of links that the vanilla client will display in the menu available
/// from the pause menu. Link labels can be built-in or custom (i.e., any text).
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Server_Links_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The inbound [`ClientInformationPacket`]. (Placeholder)
///
/// Sent when the player connects, or when settings are changed.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Client_Information_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct ClientInformationPacket;

impl Packet for ClientInformationPacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl InboundPacket for ClientInformationPacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`CookieResponsePacket`]. (Placeholder)
///
/// Response to a Cookie Request (configuration) from the server. The vanilla server only accepts
/// responses of up to 5 kiB in size.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Response_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct CookieResponsePacket;

impl Packet for CookieResponsePacket {
    fn get_packet_id() -> usize {
        0x01
    }
}

impl InboundPacket for CookieResponsePacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`InPluginMessagePacket`]. (Placeholder)
///
/// Mods and plugins can use this to send their data. Minecraft itself uses some plugin channels.
/// These internal channels are in the minecraft namespace. More documentation on this:
/// https://dinnerbone.com/blog/2012/01/13/minecraft-plugin-channels-messaging/
///
/// Note that the length of Data is known only from the packet length, since the packet has no length
/// field of any kind.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Serverbound_Plugin_Message_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct InPluginMessagePacket;

impl Packet for InPluginMessagePacket {
    fn get_packet_id() -> usize {
        0x02
    }
}

impl InboundPacket for InPluginMessagePacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`AcknowledgeFinishConfigurationPacket`]. (Placeholder)
///
/// Sent by the client to notify the server that the configuration process has finished. It is sent
/// in response to the server's Finish Configuration.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Acknowledge_Finish_Configuration)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct AcknowledgeFinishConfigurationPacket;

impl Packet for AcknowledgeFinishConfigurationPacket {
    fn get_packet_id() -> usize {
        0x03
    }
}

impl InboundPacket for AcknowledgeFinishConfigurationPacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`PongPacket`]. (Placeholder)
///
/// Response to the clientbound packet (Ping) with the same id
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Pong_(configuration))
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct PongPacket;

impl Packet for PongPacket {
    fn get_packet_id() -> usize {
        0x05
    }
}

impl InboundPacket for PongPacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`ResourcePackResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Resource_Pack_Response_(configuration))
#[derive(Debug)]
pub struct ResourcePackResponsePacket {
    pub uuid: Uuid,
    pub result: ResourcePackResult,
}

impl Packet for ResourcePackResponsePacket {
    fn get_packet_id() -> usize {
        0x06
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

    async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        // check state for any final state in the resource pack loading process
        let success = match self.result {
            ResourcePackResult::Success => true,
            ResourcePackResult::Declined
            | ResourcePackResult::DownloadFailed
            | ResourcePackResult::InvalidUrl
            | ResourcePackResult::ReloadFailed
            | ResourcePackResult::Discorded => false,
            _ => {
                // pending state, keep waiting
                return Ok(());
            }
        };

        // get and check internal state
        let Some(configuration) = &mut con.configuration else {
            return Err(Error::Generic("invalid state".to_string()));
        };

        // pop pack from list (ignoring unknown pack ids)
        let Some(pos) = configuration
            .transit_packs
            .iter()
            .position(|(uuid, _)| uuid == &self.uuid)
        else {
            return Ok(());
        };
        let (_, forced) = configuration.transit_packs.swap_remove(pos);

        // handle pack forced
        if forced && !success {
            return Err(Generic("resource pack failed".to_string()));
        }

        // handle all packs transferred
        if configuration.transit_packs.is_empty() {
            return con.transfer().await;
        }

        Ok(())
    }
}

/// The inbound [`InKnownPacksPacket`]. (Placeholder)
///
/// Informs the server of which data packs are present on the client. The client sends this in response
/// to Clientbound Known Packs. If the client specifies a pack in this packet, the server should omit
/// its contained data from the Registry Data packet.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Serverbound_Known_Packs)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct InKnownPacksPacket;

impl Packet for InKnownPacksPacket {
    fn get_packet_id() -> usize {
        0x07
    }
}

impl InboundPacket for InKnownPacksPacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AsyncReadPacket;
    use std::io::Cursor;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(TransferPacket::get_packet_id(), 0x0B);
    }

    #[tokio::test]
    async fn decode_handshake() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = TransferPacket {
            host: "test".to_string(),
            port: 1337,
        };
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

        let host = buffer.read_string().await.unwrap();
        let port = buffer.read_varint().await.unwrap();
        assert_eq!(host, packet.host);
        assert_eq!(port, packet.port);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
