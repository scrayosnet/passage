use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::StatusSupplier;
use crate::adapter::target_selection::TargetSelector;
use crate::authentication;
use crate::protocol::configuration::inbound::{
    AckFinishConfigurationPacket, ClientInformationPacket, KnownPacksPacket, PluginMessagePacket,
    PongPacket, ResourcePackResponsePacket,
};
use crate::protocol::configuration::outbound::{DisconnectPacket, TransferPacket};
use crate::protocol::configuration::{inbound, outbound};
use crate::protocol::handshaking::inbound::HandshakePacket;
use crate::protocol::login::inbound::{
    CookieResponsePacket, EncryptionResponsePacket, LoginAcknowledgedPacket, LoginStartPacket,
};
use crate::protocol::status::inbound::{PingPacket, StatusRequestPacket};
use crate::protocol::{AsyncReadPacket, AsyncWritePacket, Error, InboundPacket};
use crate::status::Protocol;
use std::io::Cursor;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, ReadBuf};
use tokio::sync::mpsc;
use tracing::{debug, trace};
use uuid::Uuid;

/// The max packet length in bytes. Larger packets are rejected.
const MAX_PACKET_LENGTH: usize = 10_000;

macro_rules! handle {
    ($packet_type:ty, $buffer:expr, $self:expr) => {{
        let packet = <$packet_type>::new_from_buffer($buffer).await?;
        trace!(packet = ?packet, "Read packet");
        packet.handle($self).await
    }}
}

#[macro_export]
macro_rules! phase {
    ($phase:expr, $expected:path, $($field:ident,)*) => {
        let $expected { $($field,)* .. } = &mut $phase else {
            return Err(Error::InvalidState {
                actual: $phase.name(),
                expected: stringify!($expected),
            });
        };
    }
}

use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use Phase::{Acknowledge, Configuration, Encryption, Handshake, Login, Status, Transfer};
pub use phase;

#[derive(Debug)]
pub enum Phase {
    Handshake {
        client_address: SocketAddr,
    },
    Status {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
    },
    Login {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
        transfer: bool,
    },
    Transfer {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
    },
    Encryption {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
        verify_token: [u8; 32],
        should_authenticate: bool,
    },
    Acknowledge {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
        should_write_auth_cookie: bool,
    },
    Configuration {
        client_address: SocketAddr,
        protocol_version: isize,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
        transit_packs: Vec<(Uuid, bool)>,
        last_keep_alive: KeepAlive,
    },
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Handshake { .. } => "Handshake",
            Status { .. } => "Status",
            Login { .. } => "Login",
            Transfer { .. } => "Transfer",
            Encryption { .. } => "Encryption",
            Acknowledge { .. } => "Acknowledge",
            Configuration { .. } => "Configuration",
        }
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug)]
pub struct KeepAlive([u64; 2]);

impl KeepAlive {
    pub fn empty() -> Self {
        KeepAlive([0, 0])
    }

    pub fn replace(&mut self, from: u64, to: u64) -> bool {
        if self.0[0] == from {
            self.0[0] = to;
            true
        } else if self.0[1] == from {
            self.0[1] = to;
            true
        } else {
            false
        }
    }
}

/// ...
pub struct Connection<S> {
    /// The connection reader
    stream: CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec>,
    /// Shutdown channel, stops main loop
    shutdown: Option<mpsc::UnboundedSender<()>>,
    /// The status supplier of the connection
    pub status_supplier: Arc<dyn StatusSupplier>,
    /// ...
    pub target_selector: Arc<dyn TargetSelector>,
    /// ...
    pub resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
    /// The current phase of the connection.
    pub phase: Phase,
    /// Auth cookie secret.
    pub auth_secret: Option<Vec<u8>>,
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
        resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
        auth_secret: Option<Vec<u8>>,
    ) -> Connection<S> {
        Self {
            stream: CipherStream::new(stream, None, None),
            shutdown: None,
            status_supplier,
            target_selector,
            resourcepack_supplier,
            phase: Handshake { client_address },
            auth_secret,
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

        // init shutdown signal
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.shutdown = Some(tx);

        // start listening for events
        loop {
            tokio::select! {
                // use biased selection such that branches are checked in order
                biased;
                // await shutdown
                _ = rx.recv() => break,
                // await next timer tick for keep-alive
                _ = interval.tick() => self.handle_tick().await?,
                // await next packet in, reading the packet size (expect fast execution)
                maybe_length = self.read_varint() => self.handle_packet(maybe_length?).await?,
            }
        }

        Ok(())
    }

    async fn handle_tick(&mut self) -> Result<(), Error> {
        let Configuration {
            last_keep_alive, ..
        } = &mut self.phase
        else {
            return Ok(());
        };

        let id = authentication::generate_keep_alive();
        if !last_keep_alive.replace(0, id) {
            self.write_packet(DisconnectPacket {
                reason: "Missed Keepalive".to_string(),
            })
            .await?;
            self.shutdown();
            return Ok(());
        }

        let packet = outbound::KeepAlivePacket::new(id);
        self.write_packet(packet).await?;

        Ok(())
    }

    async fn handle_packet(&mut self, length: usize) -> Result<(), Error> {
        // check the length of the packe for any following content
        if length == 0 || length > MAX_PACKET_LENGTH {
            debug!(
                length,
                "packet length should be between 0 and {MAX_PACKET_LENGTH}"
            );
            return Err(Error::IllegalPacketLength);
        }

        // extract the encoded packet id
        let packet_id = self.read_varint().await?;

        trace!(
            length = length,
            packet_id = packet_id,
            phase = ?self.phase,
            "Handling packet"
        );

        // split a separate reader from stream and read packet bytes (advancing stream)
        let mut buffer = vec![];
        self.take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .await?;
        let buf = &mut Cursor::new(&buffer);

        // deserialize and handle packet based on packet id and phase
        match (packet_id, &self.phase) {
            (0x00, Handshake { .. }) => handle!(HandshakePacket, buf, self),
            (0x00, Status { .. }) => handle!(StatusRequestPacket, buf, self),
            (0x01, Status { .. }) => handle!(PingPacket, buf, self),
            (0x00, Login { .. }) => handle!(LoginStartPacket, buf, self),
            (0x04, Transfer { .. }) => handle!(CookieResponsePacket, buf, self),
            (0x01, Encryption { .. }) => handle!(EncryptionResponsePacket, buf, self),
            //(0x02, Phase::Login { .. }) => handle!(LoginPluginResponsePacket, cursor, self),
            (0x03, Acknowledge { .. }) => handle!(LoginAcknowledgedPacket, buf, self),
            (0x00, Configuration { .. }) => handle!(ClientInformationPacket, buf, self),
            (0x01, Configuration { .. }) => handle!(CookieResponsePacket, buf, self),
            (0x02, Configuration { .. }) => handle!(PluginMessagePacket, buf, self),
            (0x03, Configuration { .. }) => {
                handle!(AckFinishConfigurationPacket, buf, self)
            }
            (0x04, Configuration { .. }) => handle!(inbound::KeepAlivePacket, buf, self),
            (0x05, Configuration { .. }) => handle!(PongPacket, buf, self),
            (0x06, Configuration { .. }) => {
                handle!(ResourcePackResponsePacket, buf, self)
            }
            (0x07, Configuration { .. }) => handle!(KnownPacksPacket, buf, self),
            // otherwise
            _ => {
                debug!(
                    packe_id = packet_id,
                    phase = ?self.phase,
                    "Unsupported packet in phase"
                );
                Ok(())
            }
        }
    }

    pub(crate) fn apply_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        debug!("enabling encryption");

        // get stream ciphers and wrap stream with cipher
        let (encryptor, decryptor) = authentication::create_ciphers(&shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }

    // utilities

    /// Disables reading new packets and stopping the connection
    pub fn shutdown(&mut self) {
        // send shutdown message if available
        if let Some(shutdown) = self.shutdown.take() {
            debug!("sending connection shutdown signal");
            let _ = shutdown.send(());
        }
    }

    /// Initializes the transfer
    pub async fn transfer(&mut self) -> Result<(), Error> {
        // get expected phase state
        phase!(
            self.phase,
            Configuration,
            protocol_version,
            client_address,
            server_address,
            server_port,
            user_id,
            user_name,
        );

        // select target
        let target = self
            .target_selector
            .select(
                client_address,
                (server_address, *server_port),
                *protocol_version as Protocol,
                user_name,
                user_id,
            )
            .await?;

        // disconnect if not target found
        let Some(target) = target else {
            // TODO write actual message
            self.write_packet(DisconnectPacket {
                reason: "".to_string(),
            })
            .await?;
            self.shutdown();
            return Ok(());
        };

        // create a new transfer packet and send it
        let transfer = TransferPacket {
            host: target.ip().to_string(),
            port: target.port() as usize,
        };
        debug!(packet = debug(&transfer), "sending transfer packet");
        self.write_packet(transfer).await?;

        // start graceful shutdown
        self.shutdown();

        Ok(())
    }
}

impl<S> AsyncWrite for Connection<S>
where
    S: AsyncWrite + Unpin + Send + Sync,
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
    S: AsyncRead + Unpin + Send + Sync,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }
}
