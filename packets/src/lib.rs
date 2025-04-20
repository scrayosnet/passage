#[cfg(test)]
use fake::Dummy;
use std::fmt::Debug;
use std::io::ErrorKind;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

pub mod configuration;
pub mod handshake;
pub mod login;
pub mod reader;
pub mod status;
pub mod writer;

const INITIAL_BUFFER_SIZE: usize = 48;

pub type VerifyToken = [u8; 32];

pub type VarInt = i32;

pub type VarLong = i64;

/// The internal error type for all errors related to the protocol communication.
///
/// This includes errors with the expected packets, packet contents or encoding of the exchanged fields. Errors of the
/// underlying data layer (for Byte exchange) are wrapped from the underlying IO errors. Additionally, the internal
/// timeout limits also are covered as errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An error occurred while reading or writing to the underlying byte stream.
    #[error("error reading or writing data: {0}")]
    Io(#[from] std::io::Error),

    /// The received packets is of an invalid length that we cannot process.
    #[error("illegal packets length")]
    IllegalPacketLength,

    /// The received value index cannot be mapped to an existing enum.
    #[error("illegal enum value index for {kind}: {value}")]
    IllegalEnumValue {
        /// The enum kind which was parsed.
        kind: &'static str,
        /// The value that was received.
        value: VarInt,
    },

    /// The received packets ID is not mapped to an expected packet.
    #[error("illegal packets ID: {actual} (expected {expected})")]
    IllegalPacketId {
        /// The expected value that should be present.
        expected: VarInt,
        /// The actual value that was observed.
        actual: VarInt,
    },

    /// The JSON response of a packet is incorrectly encoded (not UTF-8).
    #[error("invalid response body (invalid encoding)")]
    InvalidEncoding,

    /// Some array conversion failed.
    #[error("could not convert into array")]
    ArrayConversionFailed,
}

impl Error {
    pub fn is_connection_closed(&self) -> bool {
        let Error::Io(err) = self else {
            return false;
        };
        err.kind() == ErrorKind::UnexpectedEof
            || err.kind() == ErrorKind::ConnectionReset
            || err.kind() == ErrorKind::ConnectionAborted
            || err.kind() == ErrorKind::BrokenPipe
    }
}

/// State is the desired state that the connection should be in after the initial handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum State {
    /// Query the server information without connecting.
    Status,
    /// Log into the Minecraft server, establishing a connection.
    Login,
    /// The status s
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ResourcePackResult {
    Success,
    Declined,
    DownloadFailed,
    Accepted,
    Downloaded,
    InvalidUrl,
    ReloadFailed,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ChatMode {
    Enabled,
    CommandsOnly,
    Hidden,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub struct DisplayedSkinParts(pub u8);

impl DisplayedSkinParts {
    pub fn cape_enabled(&self) -> bool {
        self.0 & 0x01 != 0
    }

    pub fn jacket_enabled(&self) -> bool {
        self.0 & 0x02 != 0
    }

    pub fn left_sleeve_enabled(&self) -> bool {
        self.0 & 0x04 != 0
    }

    pub fn right_sleeve_enabled(&self) -> bool {
        self.0 & 0x08 != 0
    }

    pub fn left_pants_enabled(&self) -> bool {
        self.0 & 0x10 != 0
    }

    pub fn right_pants_enabled(&self) -> bool {
        self.0 & 0x20 != 0
    }

    pub fn hat_enabled(&self) -> bool {
        self.0 & 0x40 != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum MainHand {
    Left,
    Right,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(Dummy))]
pub enum ParticleStatus {
    All,
    Decreased,
    Minimal,
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

/// `WritePacket`s are packets that can be written to a buffer.
pub trait WritePacket: Packet {
    /// Writes the data from this packet into the supplied [`S`].
    fn write_to_buffer<S>(&self, buffer: &mut S) -> impl Future<Output = Result<(), Error>>
    where
        S: AsyncWrite + Unpin + Send + Sync;
}

/// `ReadPacket`s are packets that can be read from a buffer.
pub trait ReadPacket: Packet + Sized {
    /// Creates a new instance of this packet with the data from the buffer.
    fn read_from_buffer<S>(buffer: &mut S) -> impl Future<Output = Result<Self, Error>>
    where
        S: AsyncRead + Unpin + Send + Sync;
}

/// `AsyncWritePacket` allows writing a specific [`WritePacket`] to an [`AsyncWrite`].
///
/// Only [`WritePacket`s](WritePacket) can be written as only those packets are sent. There are additional
/// methods to write the data that is encoded in a Minecraft-specific manner. Their implementation is analogous to the
/// [read implementation](AsyncReadPacket).
pub trait AsyncWritePacket {
    /// Writes a [`WritePacket`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Packet_format
    fn write_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> impl Future<Output = Result<(), Error>>;

    /// Writes a [`VarInt`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn write_varint(&mut self, int: VarInt) -> impl Future<Output = Result<(), Error>>;

    /// Writes a [`VarLong`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn write_varlong(&mut self, int: VarLong) -> impl Future<Output = Result<(), Error>>;

    /// Writes a `String` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    fn write_string(&mut self, string: &str) -> impl Future<Output = Result<(), Error>>;

    /// Writes a `Uuid` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    fn write_uuid(&mut self, uuid: &Uuid) -> impl Future<Output = Result<(), Error>>;

    /// Writes a `bool` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    fn write_bool(&mut self, bool: bool) -> impl Future<Output = Result<(), Error>>;

    /// Writes a string TextComponent onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol#Type:Text_Component
    fn write_text_component(&mut self, str: &str) -> impl Future<Output = Result<(), Error>>;

    /// Writes a vec of `u8` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    fn write_bytes(&mut self, arr: &[u8]) -> impl Future<Output = Result<(), Error>>;
}

/// `AsyncReadPacket` allows reading a specific [`ReadPacket`] from an [`AsyncWrite`].
///
/// Only [`ReadPacket`s](ReadPacket) can be read as only those packets are received. There are additional
/// methods to read the data that is encoded in a Minecraft-specific manner. Their implementation is analogous to the
/// [write implementation](AsyncWritePacket).
pub trait AsyncReadPacket {
    /// Reads the supplied [`ReadPacket`] type from this object as described in the official
    /// [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Packet_format
    fn read_packet<T: ReadPacket + Send + Sync>(
        &mut self,
    ) -> impl Future<Output = Result<T, Error>>;

    /// Reads a [`VarInt`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn read_varint(&mut self) -> impl Future<Output = Result<VarInt, Error>>;

    /// Reads a [`VarLong`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    fn read_varlong(&mut self) -> impl Future<Output = Result<VarLong, Error>>;

    /// Reads a `String` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    fn read_string(&mut self) -> impl Future<Output = Result<String, Error>>;

    /// Reads a `bool` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    fn read_bool(&mut self) -> impl Future<Output = Result<bool, Error>>;

    /// Reads a `Uuid` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    fn read_uuid(&mut self) -> impl Future<Output = Result<Uuid, Error>>;

    /// Reads a string TextComponent from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol#Type:Text_Component
    fn read_text_component(&mut self) -> impl Future<Output = Result<String, Error>>;

    /// Reads a vec of `u8` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    fn read_bytes(&mut self) -> impl Future<Output = Result<Vec<u8>, Error>>;
}

#[cfg(test)]
mod tests {
    use crate::{ReadPacket, VarInt, WritePacket};
    use fake::{Dummy, Fake, Faker};
    use std::fmt::Debug;
    use std::io::Cursor;

    pub async fn assert_packet<T>(packet_id: VarInt)
    where
        T: PartialEq + Eq + Dummy<Faker> + ReadPacket + WritePacket + Send + Sync + Debug + Clone,
    {
        // generate data
        let expected: T = Faker.fake();

        // write packets
        let mut writer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        expected
            .write_to_buffer(&mut writer)
            .await
            .expect("failed to write packets");

        // read packets
        let mut reader: Cursor<Vec<u8>> = Cursor::new(writer.into_inner());
        let actual = T::read_from_buffer(&mut reader)
            .await
            .expect("failed to read packets");

        assert_eq!(T::ID, packet_id, "mismatching packet id");
        assert_eq!(expected, actual);
        assert_eq!(
            reader.position() as usize,
            reader.get_ref().len(),
            "there are remaining bytes in the buffer"
        );
    }
}
