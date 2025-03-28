use crate::authentication;
use crate::authentication::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use crate::protocol::Error::Generic;
use crate::protocol::configuration::{
    AcknowledgeFinishConfigurationPacket, AddResourcePackPacket, ClientInformationPacket,
    InKnownPacksPacket, InPluginMessagePacket, KeepAlivePacket, PongPacket,
    ResourcePackResponsePacket, TransferPacket,
};
use crate::protocol::handshaking::HandshakePacket;
use crate::protocol::login::{
    CookieResponsePacket, EncryptionResponsePacket, LoginAcknowledgedPacket,
    LoginPluginResponsePacket, LoginStartPacket,
};
use crate::protocol::status::{PingPacket, StatusRequestPacket};
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
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, ReadBuf};
use tracing::{debug, info, warn};
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
                info!(packet = ?packet, "Read packet");
                packet.handle($self).await
            })*,
            _ => {
                warn!(packe_id = $id, phase = ?$ph, "Unsupported packet in phase");
                Ok(())
            }
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
pub struct Configuration {
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

        // start listening for events
        loop {
            tokio::select! {
                // use biased selection such that the timer is chosen first
                biased;
                // await next timer tick for keep-alive
                _ = interval.tick() => self.handle_tick().await?,
                // await next packet in, reading the packet size (expect fast execution)
                maybe_length = self.read_varint() => self.handle_packet(maybe_length?).await?,
            }
        }
    }

    async fn handle_tick(&mut self) -> Result<(), Error> {
        if self.phase != Phase::Configuration {
            return Ok(());
        }

        let id = authentication::generate_keep_alive();

        if !self.last_keep_alive.replace(0, id) {
            return Err(Generic("keep alive not empty".to_string()));
        }

        let packet = KeepAlivePacket::new(id);
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
        let mut cursor = Cursor::new(&buffer);

        // deserialize and handle packet based on packet id and phase
        packets! {
            (packet_id, phase, self, cursor)
            // handshake phase
            (0x00, Phase::Handshake) => HandshakePacket,
            // status phase
            (0x00, Phase::Status) => StatusRequestPacket,
            (0x01, Phase::Status) => PingPacket,
            // login phase
            (0x00, Phase::Login) => LoginStartPacket,
            (0x01, Phase::Login) => EncryptionResponsePacket,
            (0x02, Phase::Login) => LoginPluginResponsePacket,
            (0x03, Phase::Login) => LoginAcknowledgedPacket,
            (0x04, Phase::Login) => CookieResponsePacket,
            // configuration phase
            (0x00, Phase::Configuration) => ClientInformationPacket,
            (0x01, Phase::Configuration) => CookieResponsePacket,
            (0x02, Phase::Configuration) => InPluginMessagePacket,
            (0x03, Phase::Configuration) => AcknowledgeFinishConfigurationPacket,
            (0x04, Phase::Configuration) => KeepAlivePacket,
            (0x05, Phase::Configuration) => PongPacket,
            (0x06, Phase::Configuration) => ResourcePackResponsePacket,
            (0x07, Phase::Configuration) => InKnownPacksPacket,
            // ...
            // play phase
            // (unsupported)
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

    pub async fn configure(&mut self) -> Result<(), Error> {
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

        // switch to configuration phase
        self.phase = Phase::Configuration;

        // get resource packs to load
        let packs = self
            .resource_pack_supplier
            .get_resource_packs(
                &self.client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login.user_name,
                &login.user_id,
            )
            .await?;

        // register resource packs to await
        let pack_ids = packs.iter().map(|pack| (pack.uuid, pack.forced)).collect();
        self.configuration = Some(Configuration {
            transit_packs: pack_ids,
        });

        // handle no resource packs to send
        if packs.is_empty() {
            return self.transfer().await;
        }

        // send resource packs
        for pack in packs {
            self.write_packet(AddResourcePackPacket {
                uuid: pack.uuid,
                url: pack.url,
                hash: pack.hash,
                forced: pack.forced,
                prompt_message: pack.prompt_message,
            })
            .await?;
        }

        Ok(())
    }

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
        let Some(target) = target else { return Ok(()) };

        // create a new transfer packet and send it
        let transfer = TransferPacket {
            host: target.ip().to_string(),
            port: target.port() as usize,
        };
        debug!(packet = debug(&transfer), "sending transfer packet");
        self.write_packet(transfer).await?;

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
