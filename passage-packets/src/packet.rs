use crate::configuration as config;
use crate::handshake;
use crate::login;
use crate::status;

pub enum DynPacket {
    Handshake(handshake::serverbound::HandshakePacket),

    StatusResponse(status::clientbound::StatusResponsePacket),
    StatusPong(status::clientbound::PongPacket),
    StatusRequest(status::serverbound::StatusRequestPacket),
    StatusPing(status::serverbound::PingPacket),

    LoginDisconnect(login::clientbound::DisconnectPacket),
    LoginEncryptionRequest(login::clientbound::EncryptionRequestPacket),
    LoginSuccess(login::clientbound::LoginSuccessPacket),
    LoginSetCompressionPacket(login::clientbound::SetCompressionPacket),
    LoginPluginRequest(login::clientbound::LoginPluginRequestPacket),
    LoginCookieRequest(login::clientbound::LoginPluginRequestPacket),

    LoginStart(login::serverbound::LoginStartPacket),
    LoginEncryptionResponse(login::serverbound::EncryptionResponsePacket),
    LoginPluginResponse(login::serverbound::LoginPluginResponsePacket),
    LoginAcknowledged(login::serverbound::LoginAcknowledgedPacket),
    LoginCookieResponse(login::serverbound::CookieResponsePacket),

    ConfigurationCookieRequest(config::clientbound::StoreCookiePacket),
    ConfigurationPluginMessage(config::clientbound::PluginMessagePacket),
    ConfigurationDisconnect(config::clientbound::DisconnectPacket),
    ConfigurationFinishConfiguration(config::clientbound::FinishConfigurationPacket),
    ConfigurationKeepAlive(config::clientbound::KeepAlivePacket),
    ConfigurationPing(config::clientbound::PingPacket),
    ConfigurationResetChat(config::clientbound::ResetChatPacket),
    ConfigurationRegistryData(config::clientbound::RegistryDataPacket),
    ConfigurationRemoveResourcePack(config::clientbound::RemoveResourcePackPacket),
    ConfigurationAddResourcePack(config::clientbound::AddResourcePackPacket),
    ConfigurationStoreCookie(config::clientbound::StoreCookiePacket),
    ConfigurationTransfer(config::clientbound::TransferPacket),
    ConfigurationFeatureFlags(config::clientbound::FeatureFlagsPacket),
    ConfigurationUpdateTags(config::clientbound::UpdateTagsPacket),
    ConfigurationKnownPacks(config::clientbound::KnownPacksPacket),
    ConfigurationCustomReportDetails(config::clientbound::CustomReportDetailsPacket),
    ConfigurationServerLinks(config::clientbound::ServerLinksPacket),

    ConfigurationClientInformation(config::serverbound::ClientInformationPacket),
    ConfigurationCookieResponse(config::serverbound::CookieResponsePacket),
    ConfigurationServerPluginMessage(config::serverbound::PluginMessagePacket), // TODO
    ConfigurationAcknowledgeFinishConfiguration(config::serverbound::AcknowledgeFinishConfigurationPacket),
    ConfigurationKeepAliveServer(config::serverbound::KeepAlivePacket), // TODO
    ConfigurationPong(config::serverbound::PongPacket),
    ConfigurationResourcePackResponse(config::serverbound::ResourcePackResponsePacket),
    ConfigurationKnownPacksServer(config::serverbound::KnownPacksPacket),
}
