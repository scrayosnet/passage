use crate::ProtocolVersion;
use crate::wire::{Property, Reader, WireError, Writer};
use std::fmt::Display;

/// A 32-byte random token exchanged during the encryption handshake to verify the client.
pub type VerifyToken = [u8; 32];

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

    fn decode(
        r: &mut Reader<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
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
        field: &'static str,
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
    Discorded,
}

impl Property for ResourcePackResult {
    const NAME: &'static str = "resource_pack_result";

    fn decode(
        r: &mut Reader<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
        match r.var_int(field)? {
            0 => Ok(ResourcePackResult::Success),
            1 => Ok(ResourcePackResult::Declined),
            2 => Ok(ResourcePackResult::DownloadFailed),
            3 => Ok(ResourcePackResult::Accepted),
            4 => Ok(ResourcePackResult::Downloaded),
            5 => Ok(ResourcePackResult::InvalidUrl),
            6 => Ok(ResourcePackResult::ReloadFailed),
            7 => Ok(ResourcePackResult::Discorded),
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
        field: &'static str,
    ) -> Result<(), WireError> {
        let val = match self {
            ResourcePackResult::Success => 0,
            ResourcePackResult::Declined => 1,
            ResourcePackResult::DownloadFailed => 2,
            ResourcePackResult::Accepted => 3,
            ResourcePackResult::Downloaded => 4,
            ResourcePackResult::InvalidUrl => 5,
            ResourcePackResult::ReloadFailed => 6,
            ResourcePackResult::Discorded => 7,
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

    fn decode(
        r: &mut Reader<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
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
        field: &'static str,
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
    #[must_use]
    pub fn cape_enabled(&self) -> bool {
        self.0 & 0x01 != 0
    }

    #[must_use]
    pub fn jacket_enabled(&self) -> bool {
        self.0 & 0x02 != 0
    }

    #[must_use]
    pub fn left_sleeve_enabled(&self) -> bool {
        self.0 & 0x04 != 0
    }

    #[must_use]
    pub fn right_sleeve_enabled(&self) -> bool {
        self.0 & 0x08 != 0
    }

    #[must_use]
    pub fn left_pants_enabled(&self) -> bool {
        self.0 & 0x10 != 0
    }

    #[must_use]
    pub fn right_pants_enabled(&self) -> bool {
        self.0 & 0x20 != 0
    }

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

    fn decode(
        r: &mut Reader<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
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
        field: &'static str,
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

    fn decode(
        r: &mut Reader<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self, WireError> {
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
        field: &'static str,
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
