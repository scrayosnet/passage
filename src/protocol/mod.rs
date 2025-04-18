//! This module defines and handles the Minecraft protocol and communication.
//!
//! This is necessary to exchange data with the target servers that should be probed. We only care about the packets
//! related to the [Handshaking][handshaking], [Status][status], [Login][login] and [Configuration][configuration]
//! phases and therefore only implement that part of the Minecraft protocol. The implementations may differ from the
//! official Minecraft client implementation if the observed outcome is the same and the result is reliable.
//!
//! [handshaking]: https://minecraft.wiki/w/Java_Edition_protocol#Handshaking
//! [status]: https://minecraft.wiki/w/Java_Edition_protocol#Status
//! [login]: https://minecraft.wiki/w/Java_Edition_protocol#Login
//! [configuration]: https://minecraft.wiki/w/Java_Edition_protocol#Configuration

use crate::authentication;
use fake::Dummy;
use std::fmt::Debug;
use std::io::ErrorKind;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub mod configuration;
pub mod handshaking;
pub mod login;
pub mod status;

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

    /// The received packet is of an invalid length that we cannot process.
    #[error("illegal packet length")]
    IllegalPacketLength,

    /// The received value index cannot be mapped to an existing enum.
    #[error("illegal enum value index for {kind}: {value}")]
    IllegalEnumValue {
        /// The enum kind which was parsed.
        kind: &'static str,
        /// The value that was received.
        value: VarInt,
    },

    /// The received `VarInt` cannot be correctly decoded (was formed incorrectly).
    #[error("invalid VarInt data")]
    InvalidVarInt,

    /// The received packet ID is not mapped to an expected packet.
    #[error("illegal packet ID: {actual} (expected {expected})")]
    IllegalPacketId {
        /// The expected value that should be present.
        expected: usize,
        /// The actual value that was observed.
        actual: usize,
    },

    /// The JSON response of a packet is incorrectly encoded (not UTF-8).
    #[error("invalid response body (invalid encoding)")]
    InvalidEncoding,

    /// The JSON version of a packet content could not be encoded.
    #[error("invalid struct for JSON (encoding problem)")]
    EncodingFail(#[from] serde_json::Error),

    /// Some crypto/authentication request failed.
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] authentication::Error),

    /// Some array conversion failed.
    #[error("could not convert into array")]
    ArrayConversionFailed,

    /// The packet handle was called while in an unexpected phase.
    #[error("invalid state: {actual} (expected {expected})")]
    InvalidState {
        expected: &'static str,
        actual: &'static str,
    },
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Dummy)]
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

#[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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

#[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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

#[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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

#[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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

#[derive(Debug, Clone, PartialEq, Eq, Dummy)]
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
    fn get_packet_id() -> usize;
}

/// `OutboundPacket`s are packets that are written from the serverside.
pub trait OutboundPacket: Packet {
    /// Writes the data from this packet into the supplied [`S`].
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync;
}

/// `InboundPacket`s are packets that are read and therefore are received from the serverside.
pub trait InboundPacket: Packet + Sized {
    /// Creates a new instance of this packet with the data from the buffer.
    async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync;
}

/// `AsyncWritePacket` allows writing a specific [`OutboundPacket`] to an [`AsyncWrite`].
///
/// Only [`OutboundPacket`s](OutboundPacket) can be written as only those packets are sent. There are additional
/// methods to write the data that is encoded in a Minecraft-specific manner. Their implementation is analogous to the
/// [read implementation](AsyncReadPacket).
pub trait AsyncWritePacket {
    /// Writes the supplied [`OutboundPacket`] onto this object as described in the official
    /// [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Packet_format
    async fn write_packet<T: OutboundPacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error>;

    /// Writes a [`VarInt`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn write_varint(&mut self, int: VarInt) -> Result<(), Error>;

    /// Writes a [`VarLong`] onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn write_varlong(&mut self, int: VarLong) -> Result<(), Error>;

    /// Writes a `String` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    async fn write_string(&mut self, string: &str) -> Result<(), Error>;

    /// Writes a `Uuid` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    async fn write_uuid(&mut self, uuid: &Uuid) -> Result<(), Error>;

    /// Writes a `bool` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    async fn write_bool(&mut self, bool: bool) -> Result<(), Error>;

    /// Writes a string TextComponent onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol#Type:Text_Component
    async fn write_text_component(&mut self, str: &str) -> Result<(), Error>;

    /// Writes a vec of `u8` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error>;
}

impl<W: AsyncWrite + Unpin + Send + Sync> AsyncWritePacket for W {
    async fn write_packet<T: OutboundPacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        // create a new buffer (our packets are very small)
        // TODO magic number
        let mut buffer = Vec::with_capacity(48);

        // write the packet id and the respective packet content
        buffer.write_varint(T::get_packet_id() as VarInt).await?;
        packet.write_to_buffer(&mut buffer).await?;

        // prepare a final buffer (leaving max 2 bytes for varint (packets never get that big))
        let packet_len = buffer.len();
        let mut final_buffer = Vec::with_capacity(packet_len + 2);
        final_buffer.write_varint(packet_len as VarInt).await?;
        final_buffer.extend_from_slice(&buffer);

        // send the final buffer into the stream
        self.write_all(&final_buffer).await?;

        Ok(())
    }

    async fn write_varint(&mut self, value: VarInt) -> Result<(), Error> {
        let mut value = value;
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i32::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf).await?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    async fn write_varlong(&mut self, value: VarLong) -> Result<(), Error> {
        let mut value = value;
        let mut buf = [0];
        loop {
            buf[0] = (value & 0b0111_1111) as u8;
            value = (value >> 7) & (i64::MAX >> 6);
            if value != 0 {
                buf[0] |= 0b1000_0000;
            }
            self.write_all(&buf).await?;

            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    async fn write_string(&mut self, string: &str) -> Result<(), Error> {
        self.write_varint(string.len() as VarInt).await?;
        self.write_all(string.as_bytes()).await?;

        Ok(())
    }

    async fn write_uuid(&mut self, id: &Uuid) -> Result<(), Error> {
        self.write_u128(id.as_u128()).await?;

        Ok(())
    }

    async fn write_bool(&mut self, bool: bool) -> Result<(), Error> {
        self.write_u8(bool as u8).await?;

        Ok(())
    }

    async fn write_text_component(&mut self, str: &str) -> Result<(), Error> {
        // writes a TAG_String (0x08) TextComponent
        self.write_u8(0x08).await?;
        self.write_u16(str.len() as u16).await?;
        self.write_all(str.as_bytes()).await?;

        Ok(())
    }

    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error> {
        self.write_varint(arr.len() as VarInt).await?;
        self.write_all(arr).await?;

        Ok(())
    }
}

/// `AsyncReadPacket` allows reading a specific [`InboundPacket`] from an [`AsyncWrite`].
///
/// Only [`InboundPacket`s](InboundPacket) can be read as only those packets are received. There are additional
/// methods to read the data that is encoded in a Minecraft-specific manner. Their implementation is analogous to the
/// [write implementation](AsyncWritePacket).
pub trait AsyncReadPacket {
    /// Reads the supplied [`InboundPacket`] type from this object as described in the official
    /// [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Packet_format
    async fn read_packet<T: InboundPacket + Send + Sync>(&mut self) -> Result<T, Error>;

    /// Reads a [`VarInt`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn read_varint(&mut self) -> Result<VarInt, Error>;

    /// Reads a [`VarLong`] from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn read_varlong(&mut self) -> Result<VarLong, Error>;

    /// Reads a `String` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    async fn read_string(&mut self) -> Result<String, Error>;

    /// Reads a `bool` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Boolean
    async fn read_bool(&mut self) -> Result<bool, Error>;

    /// Reads a `Uuid` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    async fn read_uuid(&mut self) -> Result<Uuid, Error>;

    /// Reads a string TextComponent from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Java_Edition_protocol#Type:Text_Component
    async fn read_text_component(&mut self) -> Result<String, Error>;

    /// Reads a vec of `u8` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    async fn read_bytes(&mut self) -> Result<Vec<u8>, Error>;
}

impl<R: AsyncRead + Unpin + Send + Sync> AsyncReadPacket for R {
    async fn read_packet<T: InboundPacket + Send + Sync>(&mut self) -> Result<T, Error> {
        // extract the length of the packet and check for any following content
        let length = self.read_varint().await?;
        if length == 0 {
            return Err(Error::IllegalPacketLength);
        }

        // extract the encoded packet id and validate if it is expected
        let packet_id = self.read_varint().await? as usize;
        let expected_packet_id = T::get_packet_id();
        if packet_id != expected_packet_id {
            return Err(Error::IllegalPacketId {
                expected: expected_packet_id,
                actual: packet_id,
            });
        }

        // split a separate reader from stream
        let mut take = self.take(length as u64);

        // convert the received buffer into our expected packet
        T::new_from_buffer(&mut take).await
    }

    async fn read_varint(&mut self) -> Result<VarInt, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..5 {
            self.read_exact(&mut buf).await?;
            ans |= ((buf[0] & 0b0111_1111) as i32) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    async fn read_varlong(&mut self) -> Result<VarLong, Error> {
        let mut buf = [0];
        let mut ans = 0;
        for i in 0..9 {
            self.read_exact(&mut buf).await?;
            ans |= ((buf[0] & 0b0111_1111) as i64) << (7 * i);
            if buf[0] & 0b1000_0000 == 0 {
                break;
            }
        }
        Ok(ans)
    }

    async fn read_string(&mut self) -> Result<String, Error> {
        let length = self.read_varint().await? as usize;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding)
    }

    async fn read_bool(&mut self) -> Result<bool, Error> {
        let bool = self.read_u8().await?;
        Ok(bool == 1u8)
    }

    async fn read_uuid(&mut self) -> Result<Uuid, Error> {
        let value = self.read_u128().await?;

        Ok(Uuid::from_u128(value))
    }

    async fn read_text_component(&mut self) -> Result<String, Error> {
        // expect a TAG_String (0x08) TextComponent
        let _tag = self.read_u8().await?;
        let len = self.read_u16().await?;

        let mut buffer = vec![0; len as usize];
        self.read_exact(&mut buffer).await?;

        String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding)
    }

    async fn read_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let length = self.read_varint().await? as usize;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        Ok(buffer)
    }
}

pub trait PacketHandler<T>: Sized {
    async fn handle(&mut self, packet: T) -> Result<(), Error>
    where
        T: Packet;
}

#[cfg(test)]
mod tests {
    use crate::protocol::{InboundPacket, OutboundPacket};
    use fake::{Dummy, Fake, Faker};
    use std::fmt::Debug;
    use std::io::Cursor;

    pub async fn assert_packet<T>(packet_id: usize)
    where
        T: PartialEq
            + Eq
            + Dummy<Faker>
            + InboundPacket
            + OutboundPacket
            + Send
            + Sync
            + Debug
            + Clone,
    {
        // generate_data
        let expected: T = Faker.fake();

        // write packet
        let mut writer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        expected
            .write_to_buffer(&mut writer)
            .await
            .expect("failed to write packet");

        // buffer
        let buffer = writer.into_inner();
        println!("packet: {:?}; buffer: {:?}", expected, buffer);

        // read packet
        let mut reader: Cursor<Vec<u8>> = Cursor::new(buffer);
        let actual = T::new_from_buffer(&mut reader)
            .await
            .expect("failed to read packet");

        assert_eq!(T::get_packet_id(), packet_id, "mismatching packet id");
        assert_eq!(expected, actual);
        assert_eq!(
            reader.position() as usize,
            reader.get_ref().len(),
            "there are remaining bytes in the buffer"
        );
    }
}
