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
use crate::connection::Connection;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

mod configuration;
pub(crate) mod handshaking;
pub(crate) mod login;
pub(crate) mod status;

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
    /// The received state index cannot be mapped to an existing state.
    #[error("illegal state index: {state}")]
    IllegalState {
        /// The state index that was received.
        state: usize,
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
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] authentication::Error),
    #[error("some generic error (placeholder)")]
    Generic(String),
}

/// State is the desired state that the connection should be in after the initial handshake.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Query the server information without connecting.
    Status,
    /// Log into the Minecraft server, establishing a connection.
    Login,
    /// The status s
    Transfer,
}

/// Phase is the phase the connection is currently in. This dictates how packet are identified.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// The handshaking phase (initial state).
    Handshake,
    /// The status phase.
    Status,
    /// The login phase.
    Login,
    /// The configuration phase.
    Configuration,
    /// The play phase (unsupported).
    Play,
}

impl From<State> for usize {
    fn from(state: State) -> Self {
        match state {
            State::Status => 1,
            State::Login => 2,
            State::Transfer => 3,
        }
    }
}

impl TryFrom<usize> for State {
    type Error = Error;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(State::Status),
            2 => Ok(State::Login),
            3 => Ok(State::Transfer),
            _ => Err(Error::IllegalState { state: value }),
        }
    }
}

/// Packets are network packets that are part of the protocol definition and identified by a context and ID.
pub trait Packet {
    /// Returns the defined ID of this network packet.
    fn get_packet_id() -> usize;

    /// Returns the defined phase of this network packet.
    fn get_phase() -> Phase;
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

    async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync;
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
    async fn write_packet<T: OutboundPacket + Send + Sync>(
        &mut self,
        packet: T,
    ) -> Result<(), Error>;

    /// Writes a `VarInt` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn write_varint(&mut self, int: usize) -> Result<(), Error>;

    /// Writes a `String` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    async fn write_string(&mut self, string: &str) -> Result<(), Error>;

    /// Writes a `Uuid` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    async fn write_uuid(&mut self, uuid: &Uuid) -> Result<(), Error>;

    /// Writes a vec of `u8` onto this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:Prefixed_Array
    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error>;
}

impl<W: AsyncWrite + Unpin + Send + Sync> AsyncWritePacket for W {
    async fn write_packet<T: OutboundPacket + Send + Sync>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        // create a new buffer (our packets are very small)
        let mut buffer = Vec::with_capacity(48);

        // write the packet id and the respective packet content
        buffer.write_varint(T::get_packet_id()).await?;
        packet.write_to_buffer(&mut buffer).await?;

        // prepare a final buffer (leaving max 2 bytes for varint (packets never get that big))
        let packet_len = buffer.len();
        let mut final_buffer = Vec::with_capacity(packet_len + 2);
        final_buffer.write_varint(packet_len).await?;
        final_buffer.extend_from_slice(&buffer);

        // send the final buffer into the stream
        self.write_all(&final_buffer).await?;

        Ok(())
    }

    async fn write_varint(&mut self, value: usize) -> Result<(), Error> {
        let mut int = (value as u64) & 0xFFFF_FFFF;
        let mut written = 0;
        let mut buffer = [0; 5];
        loop {
            let temp = (int & 0b0111_1111) as u8;
            int >>= 7;
            if int != 0 {
                buffer[written] = temp | 0b1000_0000;
            } else {
                buffer[written] = temp;
            }
            written += 1;
            if int == 0 {
                break;
            }
        }
        self.write_all(&buffer[0..written]).await?;

        Ok(())
    }

    async fn write_string(&mut self, string: &str) -> Result<(), Error> {
        self.write_varint(string.len()).await?;
        self.write_all(string.as_bytes()).await?;

        Ok(())
    }

    async fn write_uuid(&mut self, id: &Uuid) -> Result<(), Error> {
        self.write_u128(id.as_u128()).await?;

        Ok(())
    }

    async fn write_bytes(&mut self, arr: &[u8]) -> Result<(), Error> {
        self.write_varint(arr.len()).await?;
        self.write_all(arr).await?;

        Ok(())
    }
}

/// `AsyncReadPacket` allows reading a specific [`InboundPacket`] from an [`AsyncWrite`].
///
/// Only [`InboundPacket`s](InboundPacket) can be read as only those packets are received. There are additional
/// methods to read the data that is encoded in a Minecraft-specific manner. Their implementation is analogous to the
/// [write implementation](AsyncWritePacket).
pub(crate) trait AsyncReadPacket {
    /// Reads the supplied [`InboundPacket`] type from this object as described in the official
    /// [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Packet_format
    async fn read_packet<T: InboundPacket + Send + Sync>(&mut self) -> Result<T, Error>;

    /// Reads a `VarInt` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#VarInt_and_VarLong
    async fn read_varint(&mut self) -> Result<usize, Error>;

    /// Reads a `String` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:String
    async fn read_string(&mut self) -> Result<String, Error>;

    /// Reads a `Uuid` from this object as described in the official [protocol documentation][protocol-doc].
    ///
    /// [protocol-doc]: https://minecraft.wiki/w/Minecraft_Wiki:Projects/wiki.vg_merge/Protocol#Type:UUID
    async fn read_uuid(&mut self) -> Result<Uuid, Error>;

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
        let packet_id = self.read_varint().await?;
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

    async fn read_varint(&mut self) -> Result<usize, Error> {
        let mut read = 0;
        let mut result = 0;
        loop {
            let read_value = self.read_u8().await?;
            let value = read_value & 0b0111_1111;
            result |= (value as usize) << (7 * read);
            read += 1;
            if read > 5 {
                return Err(Error::InvalidVarInt);
            }
            if (read_value & 0b1000_0000) == 0 {
                return Ok(result);
            }
        }
    }

    async fn read_string(&mut self) -> Result<String, Error> {
        let length = self.read_varint().await?;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        String::from_utf8(buffer).map_err(|_| Error::InvalidEncoding)
    }

    async fn read_uuid(&mut self) -> Result<Uuid, Error> {
        let value = self.read_u128().await?;

        Ok(Uuid::from_u128(value))
    }

    async fn read_bytes(&mut self) -> Result<Vec<u8>, Error> {
        let length = self.read_varint().await?;

        let mut buffer = vec![0; length];
        self.read_exact(&mut buffer).await?;

        Ok(buffer)
    }
}
