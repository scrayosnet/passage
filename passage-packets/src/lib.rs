#[cfg(test)]
use fake::Dummy;
use std::fmt::{Debug, Display};

// re-export fastnbt
pub use fastnbt;

pub mod codec;
pub mod configuration;
pub mod error;
pub mod handshake;
pub mod login;
pub mod metrics;
pub mod reader;
pub mod status;
pub mod writer;

pub use crate::error::{Error, Result};

/// A 32-byte random token exchanged during the encryption handshake to verify the client.
pub type VerifyToken = [u8; 32];

/// Variable-length integer as defined by the Minecraft protocol (encoded as 1–5 bytes on the wire).
pub type VarInt = i32;

/// Variable-length long as defined by the Minecraft protocol (encoded as 1–10 bytes on the wire).
pub type VarLong = i64;

/// State is the desired state that the connection should be in after the initial handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum State {
    /// Query the server information without connecting.
    Status,
    /// Log into the Minecraft server, establishing a connection.
    Login,
    /// Transfer the client to another server using the Minecraft transfer packet.
    Transfer,
}

impl From<State> for VarInt {
    fn from(state: State) -> Self {
        match state {
            State::Status => 1,
            State::Login => 2,
            State::Transfer => 3,
        }
    }
}

impl TryFrom<VarInt> for State {
    type Error = Error;

    fn try_from(value: VarInt) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(State::Status),
            2 => Ok(State::Login),
            3 => Ok(State::Transfer),
            _ => Err(Error::IllegalEnumValue {
                kind: "State",
                value,
            }),
        }
    }
}

/// Result reported by the client after being sent a resource pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ResourcePackResult {
    /// The resource pack was applied successfully.
    Success,
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

impl From<ResourcePackResult> for VarInt {
    fn from(result: ResourcePackResult) -> Self {
        match result {
            ResourcePackResult::Success => 0,
            ResourcePackResult::Declined => 1,
            ResourcePackResult::DownloadFailed => 2,
            ResourcePackResult::Accepted => 3,
            ResourcePackResult::Downloaded => 4,
            ResourcePackResult::InvalidUrl => 5,
            ResourcePackResult::ReloadFailed => 6,
            ResourcePackResult::Discorded => 7,
        }
    }
}

impl TryFrom<VarInt> for ResourcePackResult {
    type Error = Error;

    fn try_from(value: VarInt) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ResourcePackResult::Success),
            1 => Ok(ResourcePackResult::Declined),
            2 => Ok(ResourcePackResult::DownloadFailed),
            3 => Ok(ResourcePackResult::Accepted),
            4 => Ok(ResourcePackResult::Downloaded),
            5 => Ok(ResourcePackResult::InvalidUrl),
            6 => Ok(ResourcePackResult::ReloadFailed),
            7 => Ok(ResourcePackResult::Discorded),
            _ => Err(Error::IllegalEnumValue {
                kind: "ResourcePackResult",
                value,
            }),
        }
    }
}

/// The client's preferred chat visibility mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ChatMode {
    /// All chat messages are shown.
    Enabled,
    /// Only command feedback is shown; player chat is hidden.
    CommandsOnly,
    /// All chat is hidden.
    Hidden,
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

impl From<ChatMode> for VarInt {
    fn from(value: ChatMode) -> Self {
        match value {
            ChatMode::Enabled => 0,
            ChatMode::CommandsOnly => 1,
            ChatMode::Hidden => 2,
        }
    }
}

impl TryFrom<VarInt> for ChatMode {
    type Error = Error;

    fn try_from(value: VarInt) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ChatMode::Enabled),
            1 => Ok(ChatMode::CommandsOnly),
            2 => Ok(ChatMode::Hidden),
            _ => Err(Error::IllegalEnumValue {
                kind: "ChatMode",
                value,
            }),
        }
    }
}

/// Bitmask describing which skin layers the client has enabled.
///
/// Each accessor tests the corresponding bit from the byte reported by the client in the
/// `ClientInformation` packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
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
#[cfg_attr(test, derive(Dummy))]
pub enum MainHand {
    /// Left-handed.
    Left,
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

impl From<MainHand> for VarInt {
    fn from(value: MainHand) -> Self {
        match value {
            MainHand::Left => 0,
            MainHand::Right => 1,
        }
    }
}

impl TryFrom<VarInt> for MainHand {
    type Error = Error;

    fn try_from(value: VarInt) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(MainHand::Left),
            1 => Ok(MainHand::Right),
            _ => Err(Error::IllegalEnumValue {
                kind: "MainHand",
                value,
            }),
        }
    }
}

/// The client's preferred particle rendering level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ParticleStatus {
    /// All particles are rendered.
    All,
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

impl From<ParticleStatus> for VarInt {
    fn from(value: ParticleStatus) -> Self {
        match value {
            ParticleStatus::All => 0,
            ParticleStatus::Decreased => 1,
            ParticleStatus::Minimal => 2,
        }
    }
}

impl TryFrom<VarInt> for ParticleStatus {
    type Error = Error;

    fn try_from(value: VarInt) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ParticleStatus::All),
            1 => Ok(ParticleStatus::Decreased),
            2 => Ok(ParticleStatus::Minimal),
            _ => Err(Error::IllegalEnumValue {
                kind: "ParticleStatus",
                value,
            }),
        }
    }
}

/// Packets are network packets that are part of the protocol definition and identified by a context and ID.
pub trait Packet {
    /// Returns the defined ID of this network packet.
    const ID: VarInt;
}

#[cfg(test)]
pub(crate) mod tests {
    use super::reader::ReadPacket;
    use super::writer::WritePacket;
    use crate::VarInt;
    use fake::{Dummy, Fake, Faker};
    use std::fmt::Debug;
    use std::io::Cursor;

    pub fn assert_packet<T>(packet_id: VarInt)
    where
        T: PartialEq + Eq + Dummy<Faker> + ReadPacket + WritePacket + Send + Sync + Debug + Clone,
    {
        // generate data
        let expected: T = Faker.fake();

        // write packets
        let mut writer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        expected
            .write_packet(&mut writer)
            .expect("failed to write packets");

        // read packets
        let mut reader: Cursor<Vec<u8>> = Cursor::new(writer.into_inner());
        let actual = T::read_packet(&mut reader).expect("failed to read packets");

        assert_eq!(T::ID, packet_id, "mismatching packet id");
        assert_eq!(expected, actual);
        assert_eq!(
            reader.position() as usize,
            reader.get_ref().len(),
            "there are remaining bytes in the buffer"
        );
    }
}
