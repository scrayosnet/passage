//! The packets of the configuration phase, which is where resource packs, cookies and transfers
//! happen. Everything here is anchored at 1.20.5, the oldest version whose IDs these are.

use crate::common::{
    ChatMode, DisplayedSkinParts, KnownPack, MAX_CODE_OF_CONDUCT_LEN, MAX_COOKIE_LEN, MAX_FEATURES,
    MAX_HASH_LEN, MAX_IDENTIFIER_LEN, MAX_KNOWN_PACKS, MAX_LOCALE_LEN, MAX_REGISTRY_ENTRIES,
    MAX_REPORT_DETAILS, MAX_SERVER_LINKS, MAX_TAGS, MAX_URL_LEN, MainHand, Nbt, ParticleStatus,
    RegistryEntry, ReportDetail, ResourcePackResult, ServerLink, TagRegistry, TextComponent,
    VarInt, versions,
};
use crate::wire::{Reader, WireError, Writer};
use crate::{Packet, Phase, ProtocolVersion};
use bytes::Bytes;
use bytestring::ByteString;
use uuid::Uuid;

/// The [`ServerCookieRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Request)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerCookieRequestPacket {
    /// The identifier of the cookie.
    pub key: ByteString,
}

impl Packet for ServerCookieRequestPacket {
    const NAME: &'static str = "server::cookie_request";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x00)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            key: r.string("key", MAX_IDENTIFIER_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("key", &self.key)?;
        Ok(())
    }
}

/// The [`ServerCustomPayloadPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Plugin_Message_(clientbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerCustomPayloadPacket {
    /// The plugin channel the data belongs to.
    pub channel: ByteString,
    /// The channel-specific data, which runs to the end of the packet.
    pub data: Bytes,
}

impl Packet for ServerCustomPayloadPacket {
    const NAME: &'static str = "server::custom_payload";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x01)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            channel: r.string("channel", MAX_IDENTIFIER_LEN)?,
            data: r.pop_rest(),
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("channel", &self.channel)?;
        w.raw(&self.data);
        Ok(())
    }
}

/// The [`ServerDisconnectPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Disconnect)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerDisconnectPacket {
    /// The component explaining why the player was turned away.
    pub reason: TextComponent,
}

impl ServerDisconnectPacket {
    /// Creates a new [`ServerDisconnectPacket`] from literal text.
    #[must_use]
    pub fn text(reason: impl Into<ByteString>) -> Self {
        Self {
            reason: TextComponent::text(reason),
        }
    }
}

impl Packet for ServerDisconnectPacket {
    const NAME: &'static str = "server::disconnect";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x02)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            reason: r.property(version, "reason")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.property(version, "reason", &self.reason)?;
        Ok(())
    }
}

/// The [`ServerFinishConfigurationPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Finish_Configuration)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerFinishConfigurationPacket;

impl Packet for ServerFinishConfigurationPacket {
    const NAME: &'static str = "server::finish_configuration";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x03)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ServerKeepAlivePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Keep_Alive_(clientbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerKeepAlivePacket {
    /// The arbitrary ID the client has to echo back.
    pub id: i64,
}

impl ServerKeepAlivePacket {
    /// Creates a new [`ServerKeepAlivePacket`] with the given ID.
    #[must_use]
    pub const fn new(id: i64) -> Self {
        Self { id }
    }
}

impl Packet for ServerKeepAlivePacket {
    const NAME: &'static str = "server::keep_alive";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self { id: r.i64("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.i64(self.id);
        Ok(())
    }
}

/// The [`ServerPingPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Ping)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerPingPacket {
    /// The arbitrary ID the client returns in its pong.
    pub id: i32,
}

impl Packet for ServerPingPacket {
    const NAME: &'static str = "server::ping";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x05)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self { id: r.i32("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.i32(self.id);
        Ok(())
    }
}

/// The [`ServerResetChatPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Reset_Chat)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerResetChatPacket;

impl Packet for ServerResetChatPacket {
    const NAME: &'static str = "server::reset_chat";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x06)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ServerRegistryDataPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Registry_Data)
#[derive(Debug, Clone, PartialEq)]
pub struct ServerRegistryDataPacket {
    /// The identifier of the registry, such as `minecraft:dimension_type`.
    pub registry: ByteString,
    /// The entries of the registry.
    pub entries: Vec<RegistryEntry>,
}

impl Packet for ServerRegistryDataPacket {
    const NAME: &'static str = "server::registry_data";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x07)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            registry: r.string("registry", MAX_IDENTIFIER_LEN)?,
            entries: r.array("entries", MAX_REGISTRY_ENTRIES, |r| {
                r.property(version, "entry")
            })?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.string("registry", &self.registry)?;
        w.array("entries", &self.entries, |w, entry| {
            w.property(version, "entry", entry)
        })?;
        Ok(())
    }
}

/// The [`ServerResourcePackPopPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Remove_Resource_Pack)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerResourcePackPopPacket {
    /// The pack to remove, or every pack if absent.
    pub uuid: Option<Uuid>,
}

impl Packet for ServerResourcePackPopPacket {
    const NAME: &'static str = "server::resource_pack_pop";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x08)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            uuid: r.optional("uuid", |r| r.uuid("uuid"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.optional(self.uuid.as_ref(), |w, uuid| {
            w.uuid(uuid);
            Ok(())
        })?;
        Ok(())
    }
}

/// The [`ServerResourcePackPushPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Add_Resource_Pack)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerResourcePackPushPacket {
    /// The identity of the pack, which the client reports back and the server removes it by.
    pub uuid: Uuid,
    /// Where the pack is downloaded from.
    pub url: ByteString,
    /// The hexadecimal SHA-1 hash of the pack file.
    pub hash: ByteString,
    /// Whether the client is kicked when it declines.
    pub forced: bool,
    /// The component shown in the prompt, if the client is to be asked.
    pub prompt_message: Option<TextComponent>,
}

impl Packet for ServerResourcePackPushPacket {
    const NAME: &'static str = "server::resource_pack_push";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x09)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            uuid: r.uuid("uuid")?,
            url: r.string("url", MAX_URL_LEN)?,
            hash: r.string("hash", MAX_HASH_LEN)?,
            forced: r.bool("forced")?,
            prompt_message: r
                .optional("prompt_message", |r| r.property(version, "prompt_message"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.uuid(&self.uuid);
        w.string("url", &self.url)?;
        w.string("hash", &self.hash)?;
        w.bool(self.forced);
        w.optional(self.prompt_message.as_ref(), |w, prompt_message| {
            w.property(version, "prompt_message", prompt_message)
        })?;
        Ok(())
    }
}

/// The [`ServerStoreCookiePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Store_Cookie)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerStoreCookiePacket {
    /// The identifier of the cookie.
    pub key: ByteString,
    /// The data to store, which survives a transfer.
    pub payload: Bytes,
}

impl Packet for ServerStoreCookiePacket {
    const NAME: &'static str = "server::store_cookie";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x0A)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            key: r.string("key", MAX_IDENTIFIER_LEN)?,
            payload: r.bytes("payload", MAX_COOKIE_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("key", &self.key)?;
        w.bytes("payload", &self.payload)?;
        Ok(())
    }
}

/// The [`ServerTransferPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerTransferPacket {
    /// The hostname or IP of the server to transfer to.
    pub host: ByteString,
    /// The port of the server to transfer to.
    pub port: u16,
}

impl Packet for ServerTransferPacket {
    const NAME: &'static str = "server::transfer";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x0B)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        let host = r.string("host", MAX_URL_LEN)?;
        let port = r.var_int("port")?;
        let port = u16::try_from(port).map_err(|_| WireError::IllegalEnumValue {
            field: "port",
            kind: "port",
        })?;
        Ok(Self { host, port })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("host", &self.host)?;
        w.var_int(i32::from(self.port));
        Ok(())
    }
}

/// The [`ServerUpdateEnabledFeaturesPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Feature_Flags)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerUpdateEnabledFeaturesPacket {
    /// The identifiers of the features to enable.
    pub features: Vec<ByteString>,
}

impl Packet for ServerUpdateEnabledFeaturesPacket {
    const NAME: &'static str = "server::update_enabled_features";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x0C)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            features: r.array("features", MAX_FEATURES, |r| {
                r.string("feature", MAX_IDENTIFIER_LEN)
            })?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.array("features", &self.features, |w, feature| {
            w.string("feature", feature)
        })?;
        Ok(())
    }
}

/// The [`ServerUpdateTagsPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Update_Tags)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerUpdateTagsPacket {
    /// The tags, grouped by the registry they belong to.
    pub registries: Vec<TagRegistry>,
}

impl Packet for ServerUpdateTagsPacket {
    const NAME: &'static str = "server::update_tags";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x0D)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            registries: r.array("registries", MAX_TAGS, |r| r.property(version, "registry"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.array("registries", &self.registries, |w, registry| {
            w.property(version, "registry", registry)
        })?;
        Ok(())
    }
}

/// The [`ServerSelectKnownPacksPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Known_Packs_(clientbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerSelectKnownPacksPacket {
    /// The data packs the server has.
    pub packs: Vec<KnownPack>,
}

impl Packet for ServerSelectKnownPacksPacket {
    const NAME: &'static str = "server::select_known_packs";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x0E)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            packs: r.array("packs", MAX_KNOWN_PACKS, |r| r.property(version, "pack"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.array("packs", &self.packs, |w, pack| {
            w.property(version, "pack", pack)
        })?;
        Ok(())
    }
}

/// The [`ServerCustomReportDetailsPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Custom_Report_Details)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerCustomReportDetailsPacket {
    /// The entries to include in any crash or disconnection report.
    pub details: Vec<ReportDetail>,
}

impl Packet for ServerCustomReportDetailsPacket {
    const NAME: &'static str = "server::custom_report_details";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_2, 0x0F)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            details: r.array("details", MAX_REPORT_DETAILS, |r| {
                r.property(version, "detail")
            })?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.array("details", &self.details, |w, detail| {
            w.property(version, "detail", detail)
        })?;
        Ok(())
    }
}

/// The [`ServerLinksPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Server_Links)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerLinksPacket {
    /// The links the client offers in its pause and disconnect screens.
    pub links: Vec<ServerLink>,
}

impl Packet for ServerLinksPacket {
    const NAME: &'static str = "server::server_links";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_2, 0x10)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            links: r.array("links", MAX_SERVER_LINKS, |r| r.property(version, "link"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.array("links", &self.links, |w, link| {
            w.property(version, "link", link)
        })?;
        Ok(())
    }
}

/// The [`ServerClearDialogPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Clear_Dialog)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerClearDialogPacket;

impl Packet for ServerClearDialogPacket {
    const NAME: &'static str = "server::clear_dialog";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_6, 0x11)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ServerShowDialogPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Show_Dialog_(configuration))
#[derive(Debug, Clone, PartialEq)]
pub struct ServerShowDialogPacket {
    /// The inline definition of the dialog to show.
    pub dialog: Nbt,
}

impl Packet for ServerShowDialogPacket {
    const NAME: &'static str = "server::show_dialog";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_6, 0x12)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            dialog: r.property(version, "dialog")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.property(version, "dialog", &self.dialog)?;
        Ok(())
    }
}

/// The [`ServerCodeOfConductPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Code_of_Conduct)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerCodeOfConductPacket {
    /// The code of conduct the player has to accept before the configuration continues.
    pub code_of_conduct: ByteString,
}

impl Packet for ServerCodeOfConductPacket {
    const NAME: &'static str = "server::code_of_conduct";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_9, 0x13)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            code_of_conduct: r.string("code_of_conduct", MAX_CODE_OF_CONDUCT_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("code_of_conduct", &self.code_of_conduct)?;
        Ok(())
    }
}

/// The [`ClientClientInformationPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Client_Information)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientClientInformationPacket {
    /// The locale of the client, such as `en_GB`.
    pub locale: ByteString,
    /// The render distance of the client, in chunks.
    pub view_distance: i8,
    /// What the client wants to see of the chat.
    pub chat_mode: ChatMode,
    /// Whether the client has chat colors enabled.
    pub chat_colors: bool,
    /// Which skin layers the client renders.
    pub displayed_skin_parts: DisplayedSkinParts,
    /// The dominant hand of the client.
    pub main_hand: MainHand,
    /// Whether the client filters the text on signs and in books.
    pub enable_text_filtering: bool,
    /// Whether the player accepts being listed in the server status sample.
    pub allow_server_listings: bool,
    /// How many particles the client renders, from 1.21.2 on.
    pub particle_status: Option<ParticleStatus>,
}

impl Packet for ClientClientInformationPacket {
    const NAME: &'static str = "client::client_information";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x00)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            locale: r.string("locale", MAX_LOCALE_LEN)?,
            view_distance: r.i8("view_distance")?,
            chat_mode: r.property(version, "chat_mode")?,
            chat_colors: r.bool("chat_colors")?,
            displayed_skin_parts: DisplayedSkinParts(r.u8("displayed_skin_parts")?),
            main_hand: r.property(version, "main_hand")?,
            enable_text_filtering: r.bool("enable_text_filtering")?,
            allow_server_listings: r.bool("allow_server_listings")?,
            particle_status: r.gated(version.at_least(versions::V1_21_2), |r| {
                r.property(version, "particle_status")
            })?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.string("locale", &self.locale)?;
        w.i8(self.view_distance);
        w.property(version, "chat_mode", &self.chat_mode)?;
        w.bool(self.chat_colors);
        w.u8(self.displayed_skin_parts.0);
        w.property(version, "main_hand", &self.main_hand)?;
        w.bool(self.enable_text_filtering);
        w.bool(self.allow_server_listings);
        if version.at_least(versions::V1_21_2) {
            let particle_status = self.particle_status.unwrap_or(ParticleStatus::All);
            w.property(version, "particle_status", &particle_status)?;
        }
        Ok(())
    }
}

/// The [`ClientCookieResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Response)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientCookieResponsePacket {
    /// The identifier of the cookie.
    pub key: ByteString,
    /// The data of the cookie, absent if the client has none stored.
    pub payload: Option<Bytes>,
}

impl Packet for ClientCookieResponsePacket {
    const NAME: &'static str = "client::cookie_response";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x01)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            key: r.string("key", MAX_IDENTIFIER_LEN)?,
            payload: r.optional("payload", |r| r.bytes("payload", MAX_COOKIE_LEN))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("key", &self.key)?;
        w.optional(self.payload.as_deref(), |w, payload| {
            w.bytes("payload", payload)
        })?;
        Ok(())
    }
}

/// The [`ClientCustomPayloadPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Plugin_Message_(serverbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientCustomPayloadPacket {
    /// The plugin channel the data belongs to.
    pub channel: ByteString,
    /// The channel-specific data, which runs to the end of the packet.
    pub data: Bytes,
}

impl Packet for ClientCustomPayloadPacket {
    const NAME: &'static str = "client::custom_payload";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x02)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            channel: r.string("channel", MAX_IDENTIFIER_LEN)?,
            data: r.pop_rest(),
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("channel", &self.channel)?;
        w.raw(&self.data);
        Ok(())
    }
}

/// The [`ClientFinishConfigurationPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Acknowledge_Finish_Configuration)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientFinishConfigurationPacket;

impl Packet for ClientFinishConfigurationPacket {
    const NAME: &'static str = "client::finish_configuration";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x03)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ClientKeepAlivePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Keep_Alive_(serverbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientKeepAlivePacket {
    /// The ID of the keep alive this answers.
    pub id: i64,
}

impl Packet for ClientKeepAlivePacket {
    const NAME: &'static str = "client::keep_alive";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self { id: r.i64("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.i64(self.id);
        Ok(())
    }
}

/// The [`ClientPongPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Pong)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientPongPacket {
    /// The ID of the ping this answers.
    pub id: i32,
}

impl Packet for ClientPongPacket {
    const NAME: &'static str = "client::pong";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x05)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self { id: r.i32("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.i32(self.id);
        Ok(())
    }
}

/// The [`ClientResourcePackPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Resource_Pack_Response)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientResourcePackPacket {
    /// The pack the client is reporting on.
    pub uuid: Uuid,
    /// What became of the pack.
    pub result: ResourcePackResult,
}

impl Packet for ClientResourcePackPacket {
    const NAME: &'static str = "client::resource_pack";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x06)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            uuid: r.uuid("uuid")?,
            result: r.property(version, "result")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.uuid(&self.uuid);
        w.property(version, "result", &self.result)?;
        Ok(())
    }
}

/// The [`ClientSelectKnownPacksPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Known_Packs_(serverbound))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientSelectKnownPacksPacket {
    /// The data packs the client already has.
    pub packs: Vec<KnownPack>,
}

impl Packet for ClientSelectKnownPacksPacket {
    const NAME: &'static str = "client::select_known_packs";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x07)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            packs: r.array("packs", MAX_KNOWN_PACKS, |r| r.property(version, "pack"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.array("packs", &self.packs, |w, pack| {
            w.property(version, "pack", pack)
        })?;
        Ok(())
    }
}

/// The [`ClientCustomClickActionPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Custom_Click_Action)
#[derive(Debug, Clone, PartialEq)]
pub struct ClientCustomClickActionPacket {
    /// The identifier of the action that was clicked.
    pub id: ByteString,
    /// The data of the action, absent if the click carried none.
    pub payload: Option<Nbt>,
}

impl Packet for ClientCustomClickActionPacket {
    const NAME: &'static str = "client::custom_click_action";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_6, 0x08)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        let id = r.string("id", MAX_IDENTIFIER_LEN)?;
        // The payload is sized rather than optional, and an empty click is a lone `TAG_End`.
        let payload = r.bytes("payload", MAX_IDENTIFIER_LEN)?;
        let payload = match payload.as_ref() {
            [] | [0x00] => None,
            _ => {
                // Shared, so the nested walk cuts its own fields out of the same frame.
                let mut sub = Reader::new(payload.clone());
                let value = sub.property(version, "payload")?;
                // The size is the peer's claim about the value; a value that does not fill it is
                // one of us misreading the other.
                sub.finish(Self::NAME)?;
                Some(value)
            }
        };
        Ok(Self { id, payload })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.string("id", &self.id)?;
        match &self.payload {
            Some(payload) => {
                let mut buf = bytes::BytesMut::new();
                Writer::new(&mut buf).property(version, "payload", payload)?;
                w.bytes("payload", buf.as_ref())?;
            }
            None => {
                w.length("payload", 1)?;
                w.u8(0x00);
            }
        }
        Ok(())
    }
}

/// The [`ClientAcceptCodeOfConductPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Accept_Code_of_Conduct)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientAcceptCodeOfConductPacket;

impl Packet for ClientAcceptCodeOfConductPacket {
    const NAME: &'static str = "client::accept_code_of_conduct";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_21_9, 0x09)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{ServerLinkLabel, ServerLinkType, Tag};
    use bytes::BytesMut;
    use fastnbt::nbt;

    /// Round trips a packet at `version` through the writer and the reader it will meet.
    fn round_trip<P: Packet + std::fmt::Debug>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = BytesMut::new();
        packet
            .encode(&mut Writer::new(&mut buf), version)
            .expect("encodes");
        let mut r = Reader::new(buf.clone().freeze());
        let decoded = P::decode(&mut r, version).expect("decodes");
        r.finish(P::NAME).expect("consumes the whole payload");
        decoded
    }

    #[test]
    fn the_packets_a_transfer_needs_round_trip() {
        // The four the router itself writes, in the order a transferred player meets them.
        let packet = ServerResourcePackPushPacket {
            uuid: Uuid::from_u128(1),
            url: "https://cdn.justchunks.net/pack.zip".into(),
            hash: "0".repeat(40).into(),
            forced: true,
            prompt_message: Some(TextComponent::text("Please install our pack")),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerKeepAlivePacket::new(i64::MIN);
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerTransferPacket {
            host: "mc.justchunks.net".into(),
            port: 25_565,
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerDisconnectPacket::text("Server full");
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn client_information_gains_a_field_at_1_21_2() {
        // Below the threshold the field is not on the wire at all, so a client that does not send
        // it must not have one read for it -- and must not be written one either.
        let packet = ClientClientInformationPacket {
            locale: "en_GB".into(),
            view_distance: 12,
            chat_mode: ChatMode::Enabled,
            chat_colors: true,
            displayed_skin_parts: DisplayedSkinParts(0x7F),
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_status: Some(ParticleStatus::Decreased),
        };
        assert_eq!(round_trip(&packet, versions::V1_21_2), packet);
        assert_eq!(
            round_trip(&packet, versions::V1_20_5).particle_status,
            None,
            "1.20.5 has no particle status",
        );
    }

    #[test]
    fn the_packets_added_after_1_20_5_do_not_exist_below_their_version() {
        // Encoding one for a client that cannot read it is an internal error rather than a frame
        // the peer has to make sense of, which is what an absent ID buys.
        assert_eq!(ServerLinksPacket::id(versions::V1_20_5), None);
        assert_eq!(ServerLinksPacket::id(versions::V1_21_2), Some(0x10));
        assert_eq!(ServerShowDialogPacket::id(versions::V1_21_2), None);
        assert_eq!(ServerShowDialogPacket::id(versions::V1_21_6), Some(0x12));
        assert_eq!(ServerCodeOfConductPacket::id(versions::V1_21_6), None);
        assert_eq!(ServerCodeOfConductPacket::id(versions::V1_21_9), Some(0x13));
        // And nothing in the phase exists before the version that introduced transfers.
        assert_eq!(ServerTransferPacket::id(ProtocolVersion::new(765)), None);
        assert_eq!(ServerTransferPacket::id(versions::V1_20_5), Some(0x0B));
    }

    #[test]
    fn a_composite_payload_round_trips_with_everything_in_place() {
        // Arrays of arrays, an optional in the middle of one, and an NBT value that is not the
        // last field: the shapes a length-prefixed walk can get wrong.
        let packet = ServerUpdateTagsPacket {
            registries: vec![TagRegistry {
                registry: "minecraft:block".into(),
                tags: vec![
                    Tag {
                        name: "minecraft:climbable".into(),
                        entries: vec![1, 2, 3],
                    },
                    Tag {
                        name: "minecraft:wool".into(),
                        entries: vec![],
                    },
                ],
            }],
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerRegistryDataPacket {
            registry: "minecraft:dimension_type".into(),
            entries: vec![
                RegistryEntry {
                    id: "minecraft:overworld".into(),
                    data: Some(Nbt(nbt!({"natural": 1i8}))),
                },
                RegistryEntry {
                    id: "minecraft:the_nether".into(),
                    data: None,
                },
            ],
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerLinksPacket {
            links: vec![
                ServerLink {
                    label: ServerLinkLabel::BuiltIn(ServerLinkType::BugReport),
                    url: "https://justchunks.net/bugs".into(),
                },
                ServerLink {
                    label: ServerLinkLabel::Custom(TextComponent::text("Discord")),
                    url: "https://justchunks.net/discord".into(),
                },
            ],
        };
        assert_eq!(round_trip(&packet, versions::V1_21_2), packet);
    }

    #[test]
    fn a_click_without_a_payload_is_a_lone_end_tag() {
        let packet = ClientCustomClickActionPacket {
            id: "justchunks:queue".into(),
            payload: None,
        };
        assert_eq!(round_trip(&packet, versions::V1_21_6), packet);

        let packet = ClientCustomClickActionPacket {
            id: "justchunks:queue".into(),
            payload: Some(Nbt(nbt!({"server": "lobby"}))),
        };
        assert_eq!(round_trip(&packet, versions::V1_21_6), packet);
    }

    #[test]
    fn a_cookie_the_client_does_not_have_costs_one_byte() {
        let packet = ClientCookieResponsePacket {
            key: "justchunks:session".into(),
            payload: None,
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ClientCookieResponsePacket {
            key: "justchunks:session".into(),
            payload: Some(Bytes::from_static(&[0x01, 0x02, 0x03])),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        // The same for the pack a server removes: absent means every pack.
        let packet = ServerResourcePackPopPacket { uuid: None };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn a_plugin_message_keeps_whatever_is_left_of_the_frame() {
        // The one field with no length of its own: it is whatever the frame did not account for.
        let packet = ClientCustomPayloadPacket {
            channel: "minecraft:brand".into(),
            data: Bytes::from_static(b"vanilla"),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }
}
