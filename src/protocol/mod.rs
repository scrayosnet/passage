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
use crate::authentication::CipherStream;
use crate::core::{StatusSupplier, TargetSelector};
use configuration::TransferPacket;
use handshaking::HandshakePacket;
use login::{
    EncryptionRequestPacket, EncryptionResponsePacket, LoginAcknowledgedPacket, LoginStartPacket,
    LoginSuccessPacket,
};
use rsa::{RsaPrivateKey, RsaPublicKey};
use status::{PingPacket, PongPacket, StatusRequestPacket, StatusResponsePacket};
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, instrument};
use uuid::Uuid;

mod configuration;
mod handshaking;
mod login;
mod status;

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
trait Packet {
    /// Returns the defined ID of this network packet.
    fn get_packet_id() -> usize;
}

/// `OutboundPacket`s are packets that are written from the serverside.
trait OutboundPacket: Packet {
    /// Writes the data from this packet into the supplied [`S`].
    async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
    where
        S: AsyncWrite + Unpin + Send + Sync;
}

/// `InboundPacket`s are packets that are read and therefore are received from the serverside.
trait InboundPacket: Packet + Sized {
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
trait AsyncWritePacket {
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
        // create a new buffer and write the packet onto it (to get the size)
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        buffer.write_varint(T::get_packet_id()).await?;
        packet.write_to_buffer(&mut buffer).await?;

        // write the length of the content (length frame encoder) and then the packet
        let inner = buffer.into_inner();
        self.write_varint(inner.len()).await?;
        self.write_all(&inner).await?;

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
trait AsyncReadPacket {
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

#[instrument(skip_all)]
pub async fn handle_client<S, SS, TS>(
    stream: &mut S,
    addr: &SocketAddr,
    keys: &(RsaPrivateKey, RsaPublicKey),
    status_supplier: Arc<SS>,
    target_selector: Arc<TS>,
) -> Result<(), Error>
where
    S: AsyncWrite + AsyncRead + Unpin + Send + Sync,
    SS: StatusSupplier,
    TS: TargetSelector,
{
    let shake = serve_handshake(stream, addr).await?;

    match &shake.state {
        State::Status => {
            serve_status(stream, &shake, status_supplier).await?;
            serve_ping(stream).await?;
        }
        State::Login | State::Transfer => {
            serve_login(stream, keys, &shake, target_selector).await?;
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct HandshakeResult {
    pub protocol: i64,
    pub state: State,
    pub client_addr: SocketAddr,
    pub server_addr: (String, u16),
}

/// Performs the status protocol exchange and returns the self-reported server status.
///
/// This receives the [`Handshake`](HandshakePacket) and the [`StatusRequest`](StatusRequestPacket) packet and sends the
/// [`StatusResponse`](StatusResponsePacket) from the server. This response is in JSON and will be supplied as-is. The
/// connection is not consumed by this operation, and the protocol allows for pings to be exchanged after the status
/// has been returned.
#[instrument(skip_all)]
pub async fn serve_handshake<S>(stream: &mut S, addr: &SocketAddr) -> Result<HandshakeResult, Error>
where
    S: AsyncWrite + AsyncRead + Unpin + Send + Sync,
{
    // await the handshake packet and read it
    debug!("awaiting and reading handshake packet");
    let handshake: HandshakePacket = stream.read_packet().await?;
    debug!(packet = debug(&handshake), "received handshake packet");

    Ok(HandshakeResult {
        protocol: handshake.protocol_version as i64,
        state: handshake.next_state,
        client_addr: *addr,
        server_addr: (handshake.server_address, handshake.server_port),
    })
}

/// Performs the status protocol exchange and returns the self-reported server status.
///
/// This receives the [`Handshake`](HandshakePacket) and the [`StatusRequest`](StatusRequestPacket) packet and sends the
/// [`StatusResponse`](StatusResponsePacket) from the server. This response is in JSON and will be supplied as-is. The
/// connection is not consumed by this operation, and the protocol allows for pings to be exchanged after the status
/// has been returned.
#[instrument(skip_all)]
pub async fn serve_status<S, SS>(
    stream: &mut S,
    handshake: &HandshakeResult,
    status_supplier: Arc<SS>,
) -> Result<(), Error>
where
    S: AsyncWrite + AsyncRead + Unpin + Send + Sync,
    SS: StatusSupplier,
{
    // await the status request and read it
    debug!("awaiting and reading status request packet");
    let request: StatusRequestPacket = stream.read_packet().await?;
    debug!(packet = debug(&request), "received status request packet");

    // get status
    let status = status_supplier
        .get_status(
            &handshake.client_addr,
            &handshake.server_addr,
            handshake.protocol,
        )
        .await?;

    // create a new status request packet and send it
    let json_response = serde_json::to_string(&status)?;

    // create a new status response packet and send it
    let request = StatusResponsePacket::new(json_response);
    debug!(packet = debug(&request), "sending status response packet");
    stream.write_packet(request).await?;

    Ok(())
}

/// Performs the ping protocol exchange and records the duration it took.
///
/// This sends the [Ping][PingPacket] and awaits the response of the [Pong][PongPacket], while recording the time it
/// takes to get a response. From this recorded RTT (Round-Trip-Time) the latency is calculated by dividing this value
/// by two. This is the most accurate way to measure the ping we can use.
#[instrument(skip_all)]
pub async fn serve_ping<S>(stream: &mut S) -> Result<(), Error>
where
    S: AsyncWrite + AsyncRead + Unpin + Send + Sync,
{
    // await the ping packet and read it
    debug!("awaiting and reading ping packet");
    let ping_request: PingPacket = stream.read_packet().await?;
    debug!(packet = debug(&ping_request), "received ping packet");

    // create a new pong packet and send it
    let pong_response = PongPacket::new(ping_request.payload);
    debug!(packet = debug(&pong_response), "sending pong packet");
    stream.write_packet(pong_response).await?;

    Ok(())
}

#[instrument(skip_all)]
pub async fn serve_login<S, TS>(
    stream: &mut S,
    keys: &(RsaPrivateKey, RsaPublicKey),
    handshake: &HandshakeResult,
    target_selector: Arc<TS>,
) -> Result<(), Error>
where
    S: AsyncWrite + AsyncRead + Unpin + Send + Sync,
    TS: TargetSelector,
{
    // await the login start packet and read it
    debug!("awaiting and reading login start packet");
    let login_start: LoginStartPacket = stream.read_packet().await?;
    debug!(packet = debug(&login_start), "received login start packet");

    // encode public key and generate verify token
    let public_key = authentication::encode_public_key(&keys.1)?;
    let verify_token = authentication::generate_token()?;

    // create a new encryption request and send it
    let encryption_request = EncryptionRequestPacket::new(public_key.clone(), verify_token, true);
    debug!(
        packet = debug(&encryption_request),
        "sending encryption request packet"
    );
    stream.write_packet(encryption_request).await?;

    // await the encryption response packet and read it
    debug!("awaiting and reading encryption response packet");
    let encryption_response: EncryptionResponsePacket = stream.read_packet().await?;
    debug!(
        packet = debug(&encryption_response),
        "received encryption response packet"
    );

    // decrypt the shared secret and verify token
    let shared_secret = authentication::decrypt(&keys.0, &encryption_response.shared_secret)?;
    let decrypted_verify_token =
        authentication::decrypt(&keys.0, &encryption_response.verify_token)?;

    // verify the token is correct
    authentication::verify_token(verify_token, &decrypted_verify_token)?;

    // get the data for login success
    let auth_response =
        authentication::authenticate_mojang(&login_start.user_name, &shared_secret, &public_key)
            .await?;

    // get stream ciphers and wrap stream with cipher
    let (encryptor, decryptor) = authentication::create_ciphers(&shared_secret)?;
    let mut stream = CipherStream::new(stream, encryptor, decryptor);

    // select target
    let target = target_selector
        .select(
            &handshake.client_addr,
            &handshake.server_addr,
            handshake.protocol,
            &auth_response.id,
            &auth_response.name,
        )
        .await?;

    // create a new login success packet and send it
    let login_success = LoginSuccessPacket::new(auth_response.id, auth_response.name);
    debug!(
        packet = debug(&login_success),
        "sending login success packet"
    );
    stream.write_packet(login_success).await?;

    // await the login acknowledged packet and read it
    debug!("awaiting and reading login acknowledged packet");
    let login_acknowledged: LoginAcknowledgedPacket = stream.read_packet().await?;
    debug!(
        packet = debug(&login_acknowledged),
        "received login acknowledged packet"
    );

    // TODO move configuration in own method (with target selection)
    // disconnect if not target found
    let Some(target) = target else { return Ok(()) };

    // create a new transfer packet and send it
    let transfer = TransferPacket::from_addr(target);
    debug!(packet = debug(&transfer), "sending transfer packet");
    stream.write_packet(transfer).await?;

    Ok(())
}
