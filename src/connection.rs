use crate::authentication;
use crate::authentication::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use crate::protocol::configuration::inbound::{
    AckFinishConfigurationPacket, ClientInformationPacket, KnownPacksPacket, PluginMessagePacket,
    PongPacket, ResourcePackResponsePacket,
};
use crate::protocol::configuration::outbound::{
    AddResourcePackPacket, DisconnectPacket, StoreCookiePacket, TransferPacket,
};
use crate::protocol::configuration::{inbound, outbound};
use crate::protocol::handshaking::inbound::HandshakePacket;
use crate::protocol::login::AUTH_COOKIE_KEY;
use crate::protocol::login::inbound::{
    CookieResponsePacket, EncryptionResponsePacket, LoginAcknowledgedPacket,
    LoginPluginResponsePacket, LoginStartPacket,
};
use crate::protocol::status::inbound::{PingPacket, StatusRequestPacket};
use crate::protocol::{AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, Phase, State};
use crate::resource_pack_supplier::ResourcePackSupplier;
use crate::status::Protocol;
use crate::status_supplier::StatusSupplier;
use crate::target_selector::TargetSelector;
use std::io::Cursor;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::io::{AsyncReadExt, ReadBuf};
use tokio::signal;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};
use uuid::Uuid;

macro_rules! handle {
    ($packet_type:ty, $buffer:expr, $self:expr) => {{
        let packet = <$packet_type>::new_from_buffer($buffer).await?;
        info!(packet = ?packet, "Read packet");
        packet.handle($self).await
    }}
}

// TODO improve state handling! Maybe Enums for every possible state? -> ensures order? / integrate phase enum?

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
pub struct Configuration {
    /// All unconfirmed resource packs (with bool if forced).
    pub transit_packs: Vec<(Uuid, bool)>,
}

pub struct KeepAlive([u64; 2]);

impl KeepAlive {
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
    pub resource_pack_supplier: Arc<dyn ResourcePackSupplier>,
    /// The client address.
    pub client_address: SocketAddr,
    /// The current phase of the connection.
    pub phase: Phase,
    /// All handshake data...
    pub handshake: Option<Handshake>,
    /// All login data...
    pub login: Option<Login>,
    /// All configuration data...
    pub configuration: Option<Configuration>,
    /// The time that the last three keep alive packet ids
    pub last_keep_alive: KeepAlive,
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
        resource_pack_supplier: Arc<dyn ResourcePackSupplier>,
    ) -> Connection<S> {
        Self {
            stream: CipherStream::new(stream, None, None),
            shutdown: None,
            status_supplier,
            target_selector,
            resource_pack_supplier,
            client_address,
            phase: Phase::Handshake,
            handshake: None,
            login: None,
            configuration: None,
            last_keep_alive: KeepAlive([0; 2]),
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
                _ = signal::ctrl_c() => break,
                // await next timer tick for keep-alive
                _ = interval.tick() => self.handle_tick().await?,
                // await next packet in, reading the packet size (expect fast execution)
                maybe_length = self.read_varint() => self.handle_packet(maybe_length?).await?,
            }
        }

        // TODO or do outside?
        // close stream cleanly
        self.stream.shutdown().await?;

        Ok(())
    }

    async fn handle_tick(&mut self) -> Result<(), Error> {
        if self.phase != Phase::Configuration {
            return Ok(());
        }

        let id = authentication::generate_keep_alive();

        if !self.last_keep_alive.replace(0, id) {
            return Err(Error::Generic("keep alive not empty".to_string()));
        }

        let packet = outbound::KeepAlivePacket::new(id);
        self.write_packet(packet).await?;

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
        // TODO only advances inner if actually read!
        let mut buffer = vec![];
        self.take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .await?;
        let cursor = &mut Cursor::new(&buffer);

        // deserialize and handle packet based on packet id and phase
        match (packet_id, phase) {
            // handshake phase
            (0x00, Phase::Handshake) => handle!(HandshakePacket, cursor, self),
            // status phase
            (0x00, Phase::Status) => handle!(StatusRequestPacket, cursor, self),
            (0x01, Phase::Status) => handle!(PingPacket, cursor, self),
            // login phase
            (0x00, Phase::Login) => handle!(LoginStartPacket, cursor, self),
            (0x01, Phase::Login) => handle!(EncryptionResponsePacket, cursor, self),
            (0x02, Phase::Login) => handle!(LoginPluginResponsePacket, cursor, self),
            (0x03, Phase::Login) => handle!(LoginAcknowledgedPacket, cursor, self),
            (0x04, Phase::Login) => handle!(CookieResponsePacket, cursor, self),
            // configuration phase
            (0x00, Phase::Configuration) => handle!(ClientInformationPacket, cursor, self),
            (0x01, Phase::Configuration) => handle!(CookieResponsePacket, cursor, self),
            (0x02, Phase::Configuration) => handle!(PluginMessagePacket, cursor, self),
            (0x03, Phase::Configuration) => handle!(AckFinishConfigurationPacket, cursor, self),
            (0x04, Phase::Configuration) => handle!(inbound::KeepAlivePacket, cursor, self),
            (0x05, Phase::Configuration) => handle!(PongPacket, cursor, self),
            (0x06, Phase::Configuration) => handle!(ResourcePackResponsePacket, cursor, self),
            (0x07, Phase::Configuration) => handle!(KnownPacksPacket, cursor, self),
            // otherwise
            _ => {
                warn!(packe_id = packet_id, phase = ?phase, "Unsupported packet in phase");
                Ok(())
            }
        }
    }

    pub(crate) fn enable_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        if self.stream.is_encrypted() {
            return Err(Error::Generic("already encrypted".to_string()));
        }

        info!("enabling encryption");

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
            let _ = shutdown.send(());
        }
    }

    /// Initializes the transfer
    pub async fn transfer(&mut self) -> Result<(), Error> {
        // get and check internal state
        let Some(handshake) = &self.handshake else {
            return Err(Error::Generic("invalid state".to_string()));
        };
        let Some(login) = &self.login else {
            return Err(Error::Generic("invalid state".to_string()));
        };
        if !login.success {
            return Err(Error::Generic("invalid state".to_string()));
        }

        // select target
        let target = self
            .target_selector
            .select(
                &self.client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login.user_id,
                &login.user_name,
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
