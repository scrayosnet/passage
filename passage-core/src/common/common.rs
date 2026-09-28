use crate::ProtocolVersion;
use crate::common::{Nbt, TextComponent};
use crate::wire::{Property, Reader, WireError, Writer};
use bytes::Bytes;
use bytestring::ByteString;
use std::fmt::Display;

/// The maximum length of an identifier (a namespaced key).
pub(crate) const MAX_IDENTIFIER_LEN: usize = 32_767;

/// The maximum length of a URL.
pub(crate) const MAX_URL_LEN: usize = 32_767;

/// The length of a resource pack hash, which is a hexadecimal SHA-1 digest.
pub(crate) const MAX_HASH_LEN: usize = 40;

/// The maximum length of a locale, such as `en_GB`.
pub(crate) const MAX_LOCALE_LEN: usize = 16;

/// The maximum size of a cookie payload, matching what the vanilla client and server accept.
pub(crate) const MAX_COOKIE_LEN: usize = 5_120;

/// The maximum length of a code of conduct.
pub(crate) const MAX_CODE_OF_CONDUCT_LEN: usize = 32_767;

/// The maximum length of a chat component sent as JSON rather than as NBT, which is the shape the
/// login phase still uses.
pub(crate) const MAX_COMPONENT_JSON_LEN: usize = 262_144;

/// The maximum length of a player name.
pub(crate) const MAX_USERNAME_LEN: usize = 16;

/// The maximum length of the server ID of the encryption handshake, which is empty in practice.
pub(crate) const MAX_SERVER_ID_LEN: usize = 20;

/// The maximum length of the blobs of the encryption handshake: the encoded public key, and the
/// shared secret and verify token encrypted under it. All three are bounded by the RSA block size,
/// with room for a key larger than the 1024 bits vanilla uses.
pub(crate) const MAX_CRYPTO_BLOB_LEN: usize = 1_024;

/// The maximum number of properties of a game profile, which carries one (`textures`) today.
pub(crate) const MAX_PROFILE_PROPERTIES: usize = 16;

/// The maximum length of a profile property's name, of its value, and of its signature.
pub(crate) const MAX_PROPERTY_NAME_LEN: usize = 64;
pub(crate) const MAX_PROPERTY_VALUE_LEN: usize = 32_767;
pub(crate) const MAX_PROPERTY_SIGNATURE_LEN: usize = 32_767;

/// The maximum length of a report detail title, and of its description.
pub(crate) const MAX_REPORT_TITLE_LEN: usize = 128;
pub(crate) const MAX_REPORT_DESCRIPTION_LEN: usize = 4_096;

/// The maximum number of entries in a registry.
pub(crate) const MAX_REGISTRY_ENTRIES: usize = 32_767;

/// The maximum number of feature flags.
pub(crate) const MAX_FEATURES: usize = 1_024;

/// The maximum number of post-processing effects, which is the same bound as the feature flags:
/// both are a bare list of identifiers with no limit of their own in the protocol.
pub(crate) const MAX_POST_EFFECTS: usize = 1_024;

/// The maximum number of registries, of tags per registry, and of IDs per tag.
pub(crate) const MAX_TAGS: usize = 32_767;

/// The maximum number of data packs either side may report.
pub(crate) const MAX_KNOWN_PACKS: usize = 1_024;

/// The maximum number of report details.
pub(crate) const MAX_REPORT_DETAILS: usize = 32;

/// The maximum number of server links.
pub(crate) const MAX_SERVER_LINKS: usize = 256;

/// A 32-byte random token exchanged during the encryption handshake to verify the client.
pub type VerifyToken = Bytes;

/// Variable-length integer as defined by the Minecraft protocol (encoded as 1–5 bytes on the wire).
pub type VarInt = i32;

/// Variable-length long as defined by the Minecraft protocol (encoded as 1–10 bytes on the wire).
pub type VarLong = i64;

/// State is the desired state that the connection should be in after the initial handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum State {
    /// Query the server information without connecting.
    Status = 1,
    /// Log into the Minecraft server, establishing a connection.
    Login,
    /// Transfer the client to another server using the Minecraft transfer packet.
    Transfer,
}

impl Property for State {
    const NAME: &'static str = "intention_state";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            1 => Ok(State::Status),
            2 => Ok(State::Login),
            3 => Ok(State::Transfer),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            State::Status => 1,
            State::Login => 2,
            State::Transfer => 3,
        };
        w.var_int(val);
        Ok(())
    }
}

/// Result reported by the client after being sent a resource pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ResourcePackResult {
    /// The resource pack was applied successfully.
    Success = 0,
    /// The client declined to download the resource pack.
    Declined,
    /// The download failed.
    DownloadFailed,
    /// The client has accepted the download, and it is in progress.
    Accepted,
    /// The resource pack was downloaded (but not yet applied).
    Downloaded,
    /// The URL provided was invalid.
    InvalidUrl,
    /// A reload of the resource pack failed.
    ReloadFailed,
    /// The resource pack was discarded.
    Discarded,
}

impl Property for ResourcePackResult {
    const NAME: &'static str = "resource_pack_result";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(ResourcePackResult::Success),
            1 => Ok(ResourcePackResult::Declined),
            2 => Ok(ResourcePackResult::DownloadFailed),
            3 => Ok(ResourcePackResult::Accepted),
            4 => Ok(ResourcePackResult::Downloaded),
            5 => Ok(ResourcePackResult::InvalidUrl),
            6 => Ok(ResourcePackResult::ReloadFailed),
            7 => Ok(ResourcePackResult::Discarded),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            ResourcePackResult::Success => 0,
            ResourcePackResult::Declined => 1,
            ResourcePackResult::DownloadFailed => 2,
            ResourcePackResult::Accepted => 3,
            ResourcePackResult::Downloaded => 4,
            ResourcePackResult::InvalidUrl => 5,
            ResourcePackResult::ReloadFailed => 6,
            ResourcePackResult::Discarded => 7,
        };
        w.var_int(val);
        Ok(())
    }
}

/// The client's preferred chat visibility mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ChatMode {
    /// All chat messages are shown.
    Enabled = 0,
    /// Only command feedback is shown; player chat is hidden.
    CommandsOnly,
    /// All chat is hidden.
    Hidden,
}

impl Property for ChatMode {
    const NAME: &'static str = "chat_mode";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(ChatMode::Enabled),
            1 => Ok(ChatMode::CommandsOnly),
            2 => Ok(ChatMode::Hidden),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            ChatMode::Enabled => 0,
            ChatMode::CommandsOnly => 1,
            ChatMode::Hidden => 2,
        };
        w.var_int(val);
        Ok(())
    }
}

impl Display for ChatMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChatMode::Enabled => write!(f, "enabled"),
            ChatMode::CommandsOnly => write!(f, "commands_only"),
            ChatMode::Hidden => write!(f, "hidden"),
        }
    }
}

/// Bitmask describing which skin layers the client has enabled.
///
/// Each accessor tests the corresponding bit from the byte reported by the client in the
/// `ClientInformation` packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayedSkinParts(pub u8);

impl DisplayedSkinParts {
    /// Whether the cape layer is shown.
    #[must_use]
    pub fn cape_enabled(&self) -> bool {
        self.0 & 0x01 != 0
    }

    /// Whether the jacket layer is shown.
    #[must_use]
    pub fn jacket_enabled(&self) -> bool {
        self.0 & 0x02 != 0
    }

    /// Whether the left sleeve layer is shown.
    #[must_use]
    pub fn left_sleeve_enabled(&self) -> bool {
        self.0 & 0x04 != 0
    }

    /// Whether the right sleeve layer is shown.
    #[must_use]
    pub fn right_sleeve_enabled(&self) -> bool {
        self.0 & 0x08 != 0
    }

    /// Whether the left trouser leg layer is shown.
    #[must_use]
    pub fn left_pants_enabled(&self) -> bool {
        self.0 & 0x10 != 0
    }

    /// Whether the right trouser leg layer is shown.
    #[must_use]
    pub fn right_pants_enabled(&self) -> bool {
        self.0 & 0x20 != 0
    }

    /// Whether the hat layer is shown.
    #[must_use]
    pub fn hat_enabled(&self) -> bool {
        self.0 & 0x40 != 0
    }
}

/// The client's dominant hand preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MainHand {
    /// Left-handed.
    Left = 0,
    /// Right-handed.
    Right,
}

impl Display for MainHand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MainHand::Left => write!(f, "left"),
            MainHand::Right => write!(f, "right"),
        }
    }
}

impl Property for MainHand {
    const NAME: &'static str = "main_hand";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(MainHand::Left),
            1 => Ok(MainHand::Right),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            MainHand::Left => 0,
            MainHand::Right => 1,
        };
        w.var_int(val);
        Ok(())
    }
}

/// The client's preferred particle rendering level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ParticleStatus {
    /// All particles are rendered.
    All = 0,
    /// Fewer particles are rendered.
    Decreased,
    /// Particles are rendered at minimum density.
    Minimal,
}

impl Display for ParticleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParticleStatus::All => write!(f, "all"),
            ParticleStatus::Decreased => write!(f, "decreased"),
            ParticleStatus::Minimal => write!(f, "minimal"),
        }
    }
}

impl Property for ParticleStatus {
    const NAME: &'static str = "particle_status";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(ParticleStatus::All),
            1 => Ok(ParticleStatus::Decreased),
            2 => Ok(ParticleStatus::Minimal),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            ParticleStatus::All => 0,
            ParticleStatus::Decreased => 1,
            ParticleStatus::Minimal => 2,
        };
        w.var_int(val);
        Ok(())
    }
}

/// One entry of a [`ServerRegistryDataPacket`](crate::packet::configuration::ServerRegistryDataPacket).
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    /// The identifier of the entry, such as `minecraft:overworld`.
    pub id: ByteString,
    /// The entry data, absent if the client is to use its own.
    pub data: Option<Nbt>,
}

impl Property for RegistryEntry {
    const NAME: &'static str = "registry_entry";

    fn decode(
        r: &mut Reader,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<Self, WireError> {
        Ok(Self {
            id: r.string("id", MAX_IDENTIFIER_LEN)?,
            data: r.optional("data", |r| r.property(version, "data"))?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("id", &self.id)?;
        w.optional(self.data.as_ref(), |w, data| {
            w.property(version, "data", data)
        })?;
        Ok(())
    }
}

/// One tag of a [`TagRegistry`]: a name and the IDs it stands for.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Tag {
    /// The identifier of the tag, such as `minecraft:climbable`.
    pub name: ByteString,
    /// The numeric IDs that carry the tag.
    pub entries: Vec<VarInt>,
}

impl Property for Tag {
    const NAME: &'static str = "tag";

    fn decode(r: &mut Reader, _: ProtocolVersion, _: &'static str) -> Result<Self, WireError> {
        Ok(Self {
            name: r.string("name", MAX_IDENTIFIER_LEN)?,
            entries: r.array("entries", MAX_TAGS, |r| r.var_int("entry"))?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("name", &self.name)?;
        w.array("entries", &self.entries, |w, entry| {
            w.var_int(*entry);
            Ok(())
        })?;
        Ok(())
    }
}

/// The tags one registry defines, as sent by a
/// [`ServerUpdateTagsPacket`](crate::packet::configuration::ServerUpdateTagsPacket).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TagRegistry {
    /// The identifier of the registry, such as `minecraft:block`.
    pub registry: ByteString,
    /// The tags defined for the registry.
    pub tags: Vec<Tag>,
}

impl Property for TagRegistry {
    const NAME: &'static str = "tag_registry";

    fn decode(
        r: &mut Reader,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<Self, WireError> {
        Ok(Self {
            registry: r.string("registry", MAX_IDENTIFIER_LEN)?,
            tags: r.array("tags", MAX_TAGS, |r| r.property(version, "tag"))?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("registry", &self.registry)?;
        w.array("tags", &self.tags, |w, tag| w.property(version, "tag", tag))?;
        Ok(())
    }
}

/// One data pack, as reported by either side of the known packs exchange.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct KnownPack {
    /// The namespace of the pack, such as `minecraft`.
    pub namespace: ByteString,
    /// The ID of the pack, such as `core`.
    pub id: ByteString,
    /// The version of the pack.
    pub version: ByteString,
}

impl Property for KnownPack {
    const NAME: &'static str = "known_pack";

    fn decode(r: &mut Reader, _: ProtocolVersion, _: &'static str) -> Result<Self, WireError> {
        Ok(Self {
            namespace: r.string("namespace", MAX_IDENTIFIER_LEN)?,
            id: r.string("id", MAX_IDENTIFIER_LEN)?,
            version: r.string("version", MAX_IDENTIFIER_LEN)?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("namespace", &self.namespace)?;
        w.string("id", &self.id)?;
        w.string("version", &self.version)?;
        Ok(())
    }
}

/// One entry of a
/// [`ServerCustomReportDetailsPacket`](crate::packet::configuration::ServerCustomReportDetailsPacket).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReportDetail {
    /// The title of the detail.
    pub title: ByteString,
    /// The description of the detail.
    pub description: ByteString,
}

impl Property for ReportDetail {
    const NAME: &'static str = "report_detail";

    fn decode(r: &mut Reader, _: ProtocolVersion, _: &'static str) -> Result<Self, WireError> {
        Ok(Self {
            title: r.string("title", MAX_REPORT_TITLE_LEN)?,
            description: r.string("description", MAX_REPORT_DESCRIPTION_LEN)?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.string("title", &self.title)?;
        w.string("description", &self.description)?;
        Ok(())
    }
}

/// One of the labels the client knows how to name itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum ServerLinkType {
    /// Where to report a bug, which the client also offers on a disconnect screen.
    BugReport = 0,
    /// The community guidelines of the server.
    CommunityGuidelines,
    /// Where to get support.
    Support,
    /// The status page of the server.
    Status,
    /// Where to leave feedback.
    Feedback,
    /// The community of the server.
    Community,
    /// The website of the server.
    Website,
    /// The forums of the server.
    Forums,
    /// The news of the server.
    News,
    /// The announcements of the server.
    Announcements,
}

impl Property for ServerLinkType {
    const NAME: &'static str = "server_link_type";

    fn decode(r: &mut Reader, _: ProtocolVersion, field: &'static str) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(ServerLinkType::BugReport),
            1 => Ok(ServerLinkType::CommunityGuidelines),
            2 => Ok(ServerLinkType::Support),
            3 => Ok(ServerLinkType::Status),
            4 => Ok(ServerLinkType::Feedback),
            5 => Ok(ServerLinkType::Community),
            6 => Ok(ServerLinkType::Website),
            7 => Ok(ServerLinkType::Forums),
            8 => Ok(ServerLinkType::News),
            9 => Ok(ServerLinkType::Announcements),
            _ => Err(WireError::IllegalEnumValue {
                field,
                kind: Self::NAME,
            }),
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            ServerLinkType::BugReport => 0,
            ServerLinkType::CommunityGuidelines => 1,
            ServerLinkType::Support => 2,
            ServerLinkType::Status => 3,
            ServerLinkType::Feedback => 4,
            ServerLinkType::Community => 5,
            ServerLinkType::Website => 6,
            ServerLinkType::Forums => 7,
            ServerLinkType::News => 8,
            ServerLinkType::Announcements => 9,
        };
        w.var_int(val);
        Ok(())
    }
}

/// How a [`ServerLink`] is labelled: either one of the client's own labels, or a component the
/// server writes itself.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ServerLinkLabel {
    /// A label the client already has a translation for.
    BuiltIn(ServerLinkType),
    /// A label the server supplies.
    Custom(TextComponent),
}

impl Property for ServerLinkLabel {
    const NAME: &'static str = "server_link_label";

    fn decode(
        r: &mut Reader,
        version: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
        if r.bool(field)? {
            Ok(Self::BuiltIn(r.property(version, field)?))
        } else {
            Ok(Self::Custom(r.property(version, field)?))
        }
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> Result<(), WireError> {
        match self {
            Self::BuiltIn(kind) => {
                w.bool(true);
                w.property(version, field, kind)
            }
            Self::Custom(label) => {
                w.bool(false);
                w.property(version, field, label)
            }
        }
    }
}

/// One link of a [`ServerLinksPacket`](crate::packet::configuration::ServerLinksPacket).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerLink {
    /// What the link is called.
    pub label: ServerLinkLabel,
    /// Where the link points.
    pub url: ByteString,
}

impl Property for ServerLink {
    const NAME: &'static str = "server_link";

    fn decode(
        r: &mut Reader,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<Self, WireError> {
        Ok(Self {
            label: r.property(version, "label")?,
            url: r.string("url", MAX_URL_LEN)?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        _: &'static str,
    ) -> Result<(), WireError> {
        w.property(version, "label", &self.label)?;
        w.string("url", &self.url)?;
        Ok(())
    }
}
