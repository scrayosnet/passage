use crate::authentication;
use crate::authentication::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use crate::protocol::handshaking::HandshakePacket;
use crate::protocol::login::{EncryptionResponsePacket, LoginAcknowledgedPacket, LoginStartPacket};
use crate::protocol::status::{PingPacket, StatusRequestPacket};
use crate::protocol::{AsyncReadPacket, Error, InboundPacket, Phase, State};
use crate::status_supplier::StatusSupplier;
use crate::target_selector::TargetSelector;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, ReadBuf};
use tracing::{info, warn};
use uuid::Uuid;

/// [`packets!`] macro expands to match statement matching a packet id and phase pair.
/// For every matched pair, a corresponding packet is deserialized and handled.
///
/// CAUTION: specifically intended for [`Connection::handle_packet`] method.
macro_rules! packets {
    { ($id:expr, $ph:expr, $self:expr, $buffer:expr) $(($packet_id:literal, $phase:path) => $packet_type:ty,)* } => {
        match ($id, $ph) {
            $(($packet_id, $phase) => {
                let packet = <$packet_type>::new_from_buffer(&mut $buffer).await?;
                packet.handle($self).await
            })*,
            _ => Ok(())
        }
    }
}

/// ...
pub struct Handshake {
    /// The pretended protocol version.
    pub protocol_version: isize,
    /// The pretended server address.
    pub server_address: String,
    /// The pretended server port.
    pub server_port: u16,
    /// The state from the handshake.
    pub state: State,
}

/// ...
pub struct Login {
    /// The generated verify token, if already generated
    pub verify_token: [u8; 32],
    /// ...
    pub user_name: String,
    /// ...
    pub user_id: Uuid,
    /// ...
    pub success: bool,
}

/// ...
pub struct Connection<S> {
    /// The connection reader
    stream: CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec>,
    /// The status supplier of the connection
    pub status_supplier: Arc<dyn StatusSupplier>,
    /// ...
    pub target_selector: Arc<dyn TargetSelector>,
    /// The client address.
    pub client_address: SocketAddr,
    /// The current phase of the connection.
    pub phase: Phase,
    /// All handshake data...
    pub handshake: Option<Handshake>,
    /// All login data...
    pub login: Option<Login>,
    /// The time that the last three keep alive packets (time and id)
    pub last_keep_alive: [(u64, u64); 3],
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite,
{
    pub(crate) fn new(
        stream: S,
        client_address: SocketAddr,
        status_supplier: Arc<dyn StatusSupplier>,
        target_selector: Arc<dyn TargetSelector>,
    ) -> Connection<S> {
        Self {
            stream: CipherStream::new(stream, None, None),
            status_supplier,
            target_selector,
            client_address,
            phase: Phase::Handshake,
            handshake: None,
            login: None,
            last_keep_alive: [(0, 0); 3],
        }
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
{
    pub async fn listen(&mut self) -> Result<(), Error> {
        // start ticker for keep-alive packets
        let mut interval = tokio::time::interval(Duration::from_secs(10));

        // start listening for events
        loop {
            tokio::select! {
                // use biased selection such that the timer is chosen first
                biased;
                // await next timer tick for keep-alive
                _ = interval.tick() => self.handle_tick().await?,
                // await next packet in, reading the packet size (expect fast execution)
                maybe_length = self.read_varint() => {
                    match maybe_length {
                        Ok(length) => {
                            info!(length = length, "Read new packet with length");
                            self.handle_packet(length).await?
                        }
                        Err(_) => {
                            // TODO connection closed?
                        }
                    }
                },
            }
        }
    }

    async fn handle_tick(&mut self) -> Result<(), Error> {
        // TODO implement me!
        // check if any expired // send keep alive
        // only if in configuration phase
        Ok(())
    }

    async fn handle_packet(&mut self, length: usize) -> Result<(), Error> {
        // check the length of the packe for any following content
        if length == 0 || length > 10_000 {
            warn!(length, "packet length should be between 0 and 10_000");
            return Err(Error::IllegalPacketLength);
        }

        // extract the encoded packet id
        let packet_id = self.read_varint().await?;

        let phase = self.phase.clone();
        info!(length = length, packet_id = packet_id, phase = ?self.phase, "Handling packet");

        // split a separate reader from stream
        let mut buffer = self.take(length as u64);

        // deserialize and handle packet based on packet id and phase
        packets! {
            (packet_id, phase, self, buffer)
            // handshake phase
            (0x00, Phase::Handshake) => HandshakePacket,
            // status phase
            (0x00, Phase::Status) => StatusRequestPacket,
            (0x01, Phase::Status) => PingPacket,
            // login phase
            (0x00, Phase::Login) => LoginStartPacket,
            (0x01, Phase::Login) => EncryptionResponsePacket,
            (0x03, Phase::Login) => LoginAcknowledgedPacket,
            // configuration phase
            // ...
            // play phase
            // (unsupported)
        }
    }

    pub(crate) fn enable_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        if self.stream.is_encrypted() {
            return Err(Error::Generic("already encrypted".to_string()));
        }

        // get stream ciphers and wrap stream with cipher
        let (encryptor, decryptor) = authentication::create_ciphers(&shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }
}

impl<S> AsyncWrite for Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

impl<S> AsyncRead for Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
