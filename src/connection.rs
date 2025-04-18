use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::{Protocol, StatusSupplier};
use crate::adapter::target_selection::TargetSelector;
use crate::authentication;
use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use Phase::{Acknowledge, Configuration, Encryption, Handshake, Login, Status, Transfer};
use packets::{
    AsyncReadPacket, AsyncWritePacket, Packet, ReadPacket, ResourcePackResult, State, VarInt,
};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::io::Cursor;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, ReadBuf};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};
use uuid::Uuid;

use packets::configuration::clientbound as conf_out;
use packets::configuration::serverbound as conf_in;
use packets::handshake::serverbound as hand_in;
use packets::login::clientbound as login_out;
use packets::login::serverbound as login_in;
use packets::status::clientbound as status_out;
use packets::status::serverbound as status_in;

/// The max packet length in bytes. Larger packets are rejected.
const MAX_PACKET_LENGTH: VarInt = 10_000;

pub const AUTH_COOKIE_KEY: &str = "passage:authentication";
pub const AUTH_COOKIE_EXPIRY_SECS: u64 = 6 * 60 * 60; // 6 hours

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An error occurred while reading or writing to the underlying byte stream.
    #[error("error reading or writing data: {0}")]
    Io(#[from] std::io::Error),

    /// The JSON version of a packet content could not be encoded.
    #[error("invalid struct for JSON (encoding problem)")]
    EncodingFail(#[from] serde_json::Error),

    /// Some crypto/authentication request failed.
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] authentication::Error),

    /// Some packet error.
    #[error("{0}")]
    PacketError(#[from] packets::Error),

    /// The packet handle was called while in an unexpected phase.
    #[error("invalid state: {actual} (expected {expected})")]
    InvalidState {
        expected: &'static str,
        actual: &'static str,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthCookie {
    pub timestamp: u64,
    pub client_addr: SocketAddr,
    pub user_name: String,
    pub user_id: Uuid,
}

trait PacketHandler<T>: Sized {
    async fn handle(&mut self, packet: T) -> Result<(), Error>
    where
        T: Packet;
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

#[derive(Debug)]
pub enum Phase {
    Handshake {
        client_address: SocketAddr,
    },
    Status {
        client_address: SocketAddr,
        protocol_version: VarInt,
        server_address: String,
        server_port: u16,
    },
    Login {
        client_address: SocketAddr,
        protocol_version: VarInt,
        server_address: String,
        server_port: u16,
        transfer: bool,
    },
    Transfer {
        client_address: SocketAddr,
        protocol_version: VarInt,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
    },
    Encryption {
        client_address: SocketAddr,
        protocol_version: VarInt,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
        verify_token: [u8; 32],
        should_authenticate: bool,
    },
    Acknowledge {
        client_address: SocketAddr,
        protocol_version: VarInt,
        server_address: String,
        server_port: u16,
        user_name: String,
        user_id: Uuid,
        should_write_auth_cookie: bool,
    },
    Configuration {
        client_address: SocketAddr,
        protocol_version: VarInt,
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
    pub fn new(
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
                // await the next timer tick for keep-alive
                _ = interval.tick() => self.handle_tick().await?,
                // await the next packet in, reading the packet size (expect fast execution)
                maybe_length = self.read_varint() => match maybe_length {
                    Ok(length) =>  self.handle_packet(length).await?,
                    Err(err) => {
                        // hide error from possible if the connection closed (eof)
                        if err.is_connection_closed() {
                            break
                        }
                        return Err(Error::PacketError(err));
                    }
                },
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
            self.write_packet(conf_out::DisconnectPacket {
                reason: "Missed Keepalive".to_string(),
            })
            .await?;
            self.shutdown();
            return Ok(());
        }

        let packet = conf_out::KeepAlivePacket::new(id);
        self.write_packet(packet).await?;

        Ok(())
    }

    async fn handle_packet(&mut self, length: VarInt) -> Result<(), Error> {
        // check the length of the packe for any following content
        if length == 0 || length > MAX_PACKET_LENGTH {
            debug!(
                length,
                "packet length should be between 0 and {MAX_PACKET_LENGTH}"
            );
            return Err(Error::PacketError(packets::Error::IllegalPacketLength));
        }

        // extract the encoded packet id
        let packet_id = self.read_varint().await?;

        trace!(
            length = length,
            packet_id = packet_id,
            phase = ?self.phase,
            "Handling packet"
        );

        // split a separate reader from the stream and read packet bytes (advancing stream)
        let mut buffer = vec![];
        self.take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .await?;
        let buf = &mut Cursor::new(&mut buffer);

        // deserialize and handle the packet based on its packet id and phase
        match (packet_id, &self.phase) {
            (0x00, Handshake { .. }) => {
                self.handle(hand_in::HandshakePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x00, Status { .. }) => {
                self.handle(status_in::StatusRequestPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x01, Status { .. }) => {
                self.handle(status_in::PingPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x00, Login { .. }) => {
                self.handle(login_in::LoginStartPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x04, Transfer { .. }) => {
                self.handle(login_in::CookieResponsePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x01, Encryption { .. }) => {
                self.handle(login_in::EncryptionResponsePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x02, Acknowledge { .. }) => {
                self.handle(login_in::LoginPluginResponsePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x03, Acknowledge { .. }) => {
                self.handle(login_in::LoginAcknowledgedPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x00, Configuration { .. }) => {
                self.handle(conf_in::ClientInformationPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x01, Configuration { .. }) => {
                self.handle(conf_in::CookieResponsePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x02, Configuration { .. }) => {
                self.handle(conf_in::PluginMessagePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x03, Configuration { .. }) => {
                self.handle(conf_in::AckFinishConfigurationPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x04, Configuration { .. }) => {
                self.handle(conf_in::KeepAlivePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x05, Configuration { .. }) => {
                self.handle(conf_in::PongPacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x06, Configuration { .. }) => {
                self.handle(conf_in::ResourcePackResponsePacket::read_from_buffer(buf).await?)
                    .await
            }
            (0x07, Configuration { .. }) => {
                self.handle(conf_in::KnownPacksPacket::read_from_buffer(buf).await?)
                    .await
            }
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
        let (encryptor, decryptor) = authentication::create_ciphers(shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }

    // utilities

    /// Disables reading new packets and stopping the connection
    pub fn shutdown(&mut self) {
        // send a shutdown message if available
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
            self.write_packet(conf_out::DisconnectPacket {
                reason: "".to_string(),
            })
            .await?;
            self.shutdown();
            return Ok(());
        };

        // create a new transfer packet and send it
        let transfer = conf_out::TransferPacket {
            host: target.ip().to_string(),
            port: target.port(),
        };
        debug!(packet = debug(&transfer), "sending transfer packet");
        self.write_packet(transfer).await?;

        // graceful shutdown
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

impl<S> PacketHandler<hand_in::HandshakePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: hand_in::HandshakePacket) -> Result<(), Error>
    where
        hand_in::HandshakePacket: Packet,
    {
        debug!(packet = ?packet, "received handshake packet");
        phase!(self.phase, Handshake, client_address,);

        // collect information
        let client_address = *client_address;
        let protocol_version = packet.protocol_version;
        let server_address = packet.server_address.to_string();
        let server_port = packet.server_port;
        let transfer = packet.next_state == State::Transfer;

        // switch to the next phase based on state
        self.phase = match &packet.next_state {
            State::Status => Status {
                client_address,
                server_address,
                server_port,
                protocol_version,
            },
            _ => Login {
                client_address,
                server_address,
                server_port,
                protocol_version,
                transfer,
            },
        };

        Ok(())
    }
}

impl<S> PacketHandler<status_in::StatusRequestPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: status_in::StatusRequestPacket) -> Result<(), Error>
    where
        status_in::StatusRequestPacket: Packet,
    {
        debug!(packet = ?packet, "received status request packet");
        phase!(
            self.phase,
            Status,
            client_address,
            server_address,
            server_port,
            protocol_version,
        );

        // get status from status supplier
        let status = self
            .status_supplier
            .get_status(
                client_address,
                (server_address, *server_port),
                *protocol_version as Protocol,
            )
            .await?;

        // create a new status request packet and send it
        let json_response = serde_json::to_string(&status)?;

        // create a new status response packet and send it
        let request = status_out::StatusResponsePacket {
            body: json_response,
        };
        debug!(packet = debug(&request), "sending status response packet");
        self.write_packet(request).await?;

        Ok(())
    }
}

impl<S> PacketHandler<status_in::PingPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: status_in::PingPacket) -> Result<(), Error>
    where
        status_in::PingPacket: Packet,
    {
        debug!(packet = ?packet, "received ping packet");
        phase!(self.phase, Status,);

        // create a new pong packet and send it
        let pong_response = status_out::PongPacket {
            payload: packet.payload,
        };
        debug!(packet = debug(&pong_response), "sending pong packet");
        self.write_packet(pong_response).await?;

        // close connection
        self.shutdown();

        Ok(())
    }
}

impl<S> PacketHandler<login_in::LoginStartPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: login_in::LoginStartPacket) -> Result<(), Error>
    where
        login_in::LoginStartPacket: Packet,
    {
        debug!(packet = ?packet, "received login start packet");
        phase!(
            self.phase,
            Phase::Login,
            client_address,
            protocol_version,
            server_address,
            server_port,
            transfer,
        );

        // handle transfer
        if *transfer && self.auth_secret.is_some() {
            // update phase and wait for cookie
            self.phase = Phase::Transfer {
                client_address: *client_address,
                protocol_version: *protocol_version,
                server_address: server_address.clone(),
                server_port: *server_port,
                user_name: packet.user_name.clone(),
                user_id: packet.user_id,
            };
            self.write_packet(login_out::CookieRequestPacket {
                key: AUTH_COOKIE_KEY.to_string(),
            })
            .await?;
            return Ok(());
        }

        // encode public key and generate verify token
        let verify_token = authentication::generate_token()?;

        // switch phase to accept encryption response
        self.phase = Phase::Encryption {
            client_address: *client_address,
            protocol_version: *protocol_version,
            server_address: server_address.clone(),
            server_port: *server_port,
            user_name: packet.user_name.clone(),
            user_id: packet.user_id,
            verify_token,
            should_authenticate: true,
        };

        // create a new encryption request and send it
        let encryption_request = login_out::EncryptionRequestPacket {
            server_id: "".to_owned(),
            public_key: authentication::ENCODED_PUB.clone(),
            verify_token,
            should_authenticate: true,
        };
        debug!(
            packet = debug(&encryption_request),
            "sending encryption request packet"
        );
        self.write_packet(encryption_request).await?;

        Ok(())
    }
}

impl<S> PacketHandler<login_in::CookieResponsePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: login_in::CookieResponsePacket) -> Result<(), Error>
    where
        login_in::CookieResponsePacket: Packet,
    {
        debug!(packet = ?packet, "received cookie response packet");

        // only supports auth cookie
        if packet.key != AUTH_COOKIE_KEY {
            return Ok(());
        }

        // get auth cookie secret
        let Some(secret) = &self.auth_secret else {
            return Ok(());
        };

        phase!(
            self.phase,
            Phase::Transfer,
            client_address,
            protocol_version,
            server_address,
            server_port,
            user_name,
            user_id,
        );

        // verify token
        let mut should_authenticate = true;
        if let Some(message) = packet.payload {
            let (ok, message) = authentication::check_sign(&message, secret);
            if ok {
                let cookie = serde_json::from_slice::<AuthCookie>(message)?;
                let expires_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time error")
                    .as_secs()
                    + AUTH_COOKIE_EXPIRY_SECS;
                if cookie.client_addr.ip() == client_address.ip() && cookie.timestamp < expires_at {
                    should_authenticate = false;

                    // update state by token
                    *user_name = cookie.user_name;
                    *user_id = cookie.user_id;
                }
            }
        }

        // encode public key and generate verify token
        let verify_token = authentication::generate_token()?;

        // switch phase to accept encryption response
        self.phase = Phase::Encryption {
            client_address: *client_address,
            protocol_version: *protocol_version,
            server_address: server_address.clone(),
            server_port: *server_port,
            user_name: user_name.clone(),
            user_id: *user_id,
            verify_token,
            should_authenticate,
        };

        // create a new encryption request and send it
        let encryption_request = login_out::EncryptionRequestPacket {
            server_id: "".to_owned(),
            public_key: authentication::ENCODED_PUB.clone(),
            verify_token,
            should_authenticate,
        };
        debug!(
            packet = debug(&encryption_request),
            "sending encryption request packet"
        );
        self.write_packet(encryption_request).await?;

        Ok(())
    }
}

impl<S> PacketHandler<login_in::EncryptionResponsePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: login_in::EncryptionResponsePacket) -> Result<(), Error>
    where
        login_in::EncryptionResponsePacket: Packet,
    {
        debug!(packet = ?packet, "received encryption response packet");
        phase!(
            self.phase,
            Phase::Encryption,
            client_address,
            protocol_version,
            server_address,
            server_port,
            user_name,
            user_id,
            verify_token,
            should_authenticate,
        );

        // decrypt the shared secret and verify token
        let shared_secret =
            authentication::decrypt(&authentication::KEY_PAIR.0, &packet.shared_secret)?;
        let decrypted_verify_token =
            authentication::decrypt(&authentication::KEY_PAIR.0, &packet.verify_token)?;

        // verify the token is correct
        authentication::verify_token(*verify_token, &decrypted_verify_token)?;

        // handle mojang auth if not handled by cookie
        if *should_authenticate {
            // get the data for login success
            let auth_response = authentication::authenticate_mojang(
                user_name,
                &shared_secret,
                &authentication::ENCODED_PUB,
            )
            .await;

            let auth_response = match auth_response {
                Ok(inner) => inner,
                Err(err) => {
                    warn!(err = ?err, "mojang auth failed");
                    // TODO write actual reason
                    self.write_packet(login_out::DisconnectPacket {
                        reason: "".to_string(),
                    })
                    .await?;
                    self.shutdown();
                    return Ok(());
                }
            };

            // update state for actual use info
            *user_name = auth_response.name;
            *user_id = auth_response.id;
        }

        // build response packet
        let login_success = login_out::LoginSuccessPacket {
            user_name: user_name.clone(),
            user_id: *user_id,
        };

        // switch to login-acknowledge phase
        self.phase = Phase::Acknowledge {
            client_address: *client_address,
            protocol_version: *protocol_version,
            server_address: server_address.clone(),
            server_port: *server_port,
            user_name: user_name.clone(),
            user_id: *user_id,
            should_write_auth_cookie: *should_authenticate,
        };

        // enable encryption for the selfnection using the shared secret
        self.apply_encryption(&shared_secret)?;

        // create a new login success packet and send it
        debug!(
            packet = debug(&login_success),
            "sending login success packet"
        );
        self.write_packet(login_success).await?;

        Ok(())
    }
}

impl<S> PacketHandler<login_in::LoginPluginResponsePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: login_in::LoginPluginResponsePacket) -> Result<(), Error>
    where
        login_in::LoginPluginResponsePacket: Packet,
    {
        Ok(())
    }
}

impl<S> PacketHandler<login_in::LoginAcknowledgedPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: login_in::LoginAcknowledgedPacket) -> Result<(), Error>
    where
        login_in::LoginAcknowledgedPacket: Packet,
    {
        debug!(packet = ?packet, "received login acknowledged packet");
        phase!(
            self.phase,
            Phase::Acknowledge,
            client_address,
            protocol_version,
            server_address,
            server_port,
            user_name,
            user_id,
            should_write_auth_cookie,
        );
        let should_write_auth_cookie = *should_write_auth_cookie;

        // generate auth cookie payload
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time error")
            .as_secs();
        let auth_payload = serde_json::to_vec(&AuthCookie {
            timestamp: now_secs,
            client_addr: *client_address,
            user_name: user_name.clone(),
            user_id: *user_id,
        })?;

        // get resource packs to load
        let packs = self
            .resourcepack_supplier
            .get_resourcepacks(
                client_address,
                (server_address, *server_port),
                *protocol_version as Protocol,
                user_name,
                user_id,
            )
            .await?;
        let pack_ids = packs.iter().map(|pack| (pack.uuid, pack.forced)).collect();

        // switch to configuration phase
        self.phase = Phase::Configuration {
            client_address: *client_address,
            protocol_version: *protocol_version,
            server_address: server_address.clone(),
            server_port: *server_port,
            user_name: user_name.clone(),
            user_id: *user_id,
            transit_packs: pack_ids,
            last_keep_alive: KeepAlive::empty(),
        };

        // store auth cookie
        if should_write_auth_cookie {
            if let Some(secret) = &self.auth_secret {
                self.write_packet(conf_out::StoreCookiePacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                    payload: authentication::sign(&auth_payload, secret),
                })
                .await?;
            }
        }

        // handle no resource packs to send
        if packs.is_empty() {
            return self.transfer().await;
        }

        // send resource packs
        for pack in packs {
            let packet = conf_out::AddResourcePackPacket {
                uuid: pack.uuid,
                url: pack.url,
                hash: pack.hash,
                forced: pack.forced,
                prompt_message: pack.prompt_message,
            };
            self.write_packet(packet).await?;
        }

        Ok(())
    }
}

impl<S> PacketHandler<conf_in::ClientInformationPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::ClientInformationPacket) -> Result<(), Error>
    where
        conf_in::ClientInformationPacket: Packet,
    {
        Ok(())
    }
}

impl<S> PacketHandler<conf_in::CookieResponsePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::CookieResponsePacket) -> Result<(), Error>
    where
        conf_in::CookieResponsePacket: Packet,
    {
        Ok(())
    }
}

impl<S> PacketHandler<conf_in::PluginMessagePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::PluginMessagePacket) -> Result<(), Error>
    where
        conf_in::PluginMessagePacket: Packet,
    {
        Ok(())
    }
}

impl<S> PacketHandler<conf_in::AckFinishConfigurationPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::AckFinishConfigurationPacket) -> Result<(), Error>
    where
        conf_in::AckFinishConfigurationPacket: Packet,
    {
        Ok(())
    }
}

impl<S> PacketHandler<conf_in::KeepAlivePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: conf_in::KeepAlivePacket) -> Result<(), Error>
    where
        conf_in::KeepAlivePacket: Packet,
    {
        debug!(packet = ?packet, "received keep alive packet");
        phase!(self.phase, Phase::Configuration, last_keep_alive,);

        if !last_keep_alive.replace(packet.id, 0) {
            debug!(id = packet.id, "keep alive packet id unknown");
        }

        Ok(())
    }
}

impl<S> PacketHandler<conf_in::PongPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::PongPacket) -> Result<(), Error>
    where
        conf_in::PongPacket: Packet,
    {
        // TODO implement me?
        Ok(())
    }
}

impl<S> PacketHandler<conf_in::ResourcePackResponsePacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, packet: conf_in::ResourcePackResponsePacket) -> Result<(), Error>
    where
        conf_in::ResourcePackResponsePacket: Packet,
    {
        debug!(packet = ?packet, "received keep alive packet");
        phase!(self.phase, Phase::Configuration, transit_packs,);

        // check the state for any final state in the resource pack loading process
        let success = match packet.result {
            ResourcePackResult::Success => true,
            ResourcePackResult::Declined
            | ResourcePackResult::DownloadFailed
            | ResourcePackResult::InvalidUrl
            | ResourcePackResult::ReloadFailed
            | ResourcePackResult::Discorded => false,
            _ => {
                // pending state, keep waiting
                return Ok(());
            }
        };

        // pop pack from the list (ignoring unknown pack ids)
        let Some(pos) = transit_packs
            .iter()
            .position(|(uuid, _)| uuid == &packet.uuid)
        else {
            return Ok(());
        };
        let (_, forced) = transit_packs.swap_remove(pos);

        // handle pack forced
        if forced && !success {
            // TODO write actual reason
            self.write_packet(conf_out::DisconnectPacket {
                reason: "".to_string(),
            })
            .await?;
            self.shutdown();
            return Ok(());
        }

        // handle all packs transferred
        if transit_packs.is_empty() {
            return self.transfer().await;
        }

        Ok(())
    }
}

impl<S> PacketHandler<conf_in::KnownPacksPacket> for Connection<S>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    async fn handle(&mut self, _packet: conf_in::KnownPacksPacket) -> Result<(), Error>
    where
        conf_in::KnownPacksPacket: Packet,
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::resourcepack::none::NoneResourcePackSupplier;
    use crate::adapter::status::ServerStatus;
    use crate::adapter::target_selection::none::NoneTargetSelector;
    use async_trait::async_trait;
    use rand::rngs::OsRng;
    use rsa::pkcs8::DecodePublicKey;
    use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
    use std::str::FromStr;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::uuid;

    struct NoneStatusSupplier;

    #[async_trait]
    impl StatusSupplier for NoneStatusSupplier {
        async fn get_status(
            &self,
            _client_addr: &SocketAddr,
            _server_addr: (&str, u16),
            protocol: Protocol,
        ) -> Result<Option<ServerStatus>, Error> {
            let mut status = ServerStatus::default();
            status.version.protocol = protocol;
            Ok(Some(status))
        }
    }

    pub fn encrypt(key: &RsaPublicKey, value: &[u8]) -> Vec<u8> {
        key.encrypt(&mut OsRng, Pkcs1v15Encrypt, value)
            .expect("encrypt failed")
    }

    #[tokio::test]
    async fn simulate_handshake() {
        // create stream
        let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
        let server_address = SocketAddr::from_str("127.0.0.1:25565").expect("invalid address");
        let (mut client_stream, server_stream) = tokio::io::duplex(1024);

        // build supplier
        let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
        let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector);
        let resourcepack_supplier: Arc<dyn ResourcepackSupplier> =
            Arc::new(NoneResourcePackSupplier);

        // build connection
        let mut server = Connection::new(
            server_stream,
            client_address,
            Arc::clone(&status_supplier),
            Arc::clone(&target_selector),
            Arc::clone(&resourcepack_supplier),
            None,
        );

        // start the server in its own thread
        let server = tokio::spawn(async move {
            server.listen().await.expect("server listen failed");
        });

        // simulate client
        client_stream
            .write_packet(hand_in::HandshakePacket {
                protocol_version: 0,
                server_address: "".to_string(),
                server_port: 0,
                next_state: State::Status,
            })
            .await
            .expect("send handshake failed");

        // simulate connection closed after the handshake packet
        drop(client_stream);

        // wait for the server to finish
        server.await.expect("server run failed");
    }

    #[tokio::test]
    async fn simulate_status() {
        // create stream
        let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
        let server_address = SocketAddr::from_str("127.0.0.1:25565").expect("invalid address");
        let (mut client_stream, server_stream) = tokio::io::duplex(1024);

        // build supplier
        let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
        let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector);
        let resourcepack_supplier: Arc<dyn ResourcepackSupplier> =
            Arc::new(NoneResourcePackSupplier);

        // build connection
        let mut server = Connection::new(
            server_stream,
            client_address,
            Arc::clone(&status_supplier),
            Arc::clone(&target_selector),
            Arc::clone(&resourcepack_supplier),
            None,
        );

        // start the server in its own thread
        let server = tokio::spawn(async move {
            server.listen().await.expect("server listen failed");
        });

        // simulate client
        client_stream
            .write_packet(hand_in::HandshakePacket {
                protocol_version: 0,
                server_address: "".to_string(),
                server_port: 0,
                next_state: State::Status,
            })
            .await
            .expect("send handshake failed");

        client_stream
            .write_packet(status_in::StatusRequestPacket)
            .await
            .expect("send status request failed");

        let status_response_packet: status_out::StatusResponsePacket = client_stream
            .read_packet()
            .await
            .expect("status response packet read failed");
        assert_eq!(
            status_response_packet.body,
            "{\"version\":{\"name\":\"JustChunks\",\"protocol\":0},\"players\":null,\"description\":null,\"favicon\":null,\"enforcesSecureChat\":null}"
        );

        client_stream
            .write_packet(status_in::PingPacket { payload: 42 })
            .await
            .expect("send ping request failed");

        let pong_packet: status_out::PongPacket = client_stream
            .read_packet()
            .await
            .expect("pong packet read failed");
        assert_eq!(pong_packet.payload, 42);

        // wait for the server to finish
        server.await.expect("server run failed");
    }

    #[tokio::test]
    async fn simulate_transfer_no_configuration() {
        let shared_secret = b"verysecuresecret";
        let user_name = "Hydrofin".to_owned();
        let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

        // create stream
        let auth_secret = b"secret".to_vec();
        let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
        let server_address = SocketAddr::from_str("127.0.0.1:25565").expect("invalid address");
        let (mut client_stream, server_stream) = tokio::io::duplex(1024);

        // build supplier
        let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
        let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector);
        let resourcepack_supplier: Arc<dyn ResourcepackSupplier> =
            Arc::new(NoneResourcePackSupplier);

        // build connection
        let mut server = Connection::new(
            server_stream,
            client_address,
            Arc::clone(&status_supplier),
            Arc::clone(&target_selector),
            Arc::clone(&resourcepack_supplier),
            Some(auth_secret.clone()),
        );

        // start the server in its own thread
        let server = tokio::spawn(async move {
            server.listen().await.expect("server listen failed");
        });

        // simulate client
        client_stream
            .write_packet(hand_in::HandshakePacket {
                protocol_version: 0,
                server_address: "".to_string(),
                server_port: 0,
                next_state: State::Transfer,
            })
            .await
            .expect("send handshake failed");

        client_stream
            .write_packet(login_in::LoginStartPacket {
                user_name: user_name.clone(),
                user_id,
            })
            .await
            .expect("send login start failed");

        let cookie_request_packet: login_out::CookieRequestPacket = client_stream
            .read_packet()
            .await
            .expect("cookie request packet read failed");
        assert_eq!(&cookie_request_packet.key, AUTH_COOKIE_KEY);

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time error")
            .as_secs();
        let auth_payload = serde_json::to_vec(&AuthCookie {
            timestamp: now_secs,
            client_addr: client_address,
            user_name: user_name.clone(),
            user_id,
        })
        .expect("auth cookie serialization failed");

        client_stream
            .write_packet(login_in::CookieResponsePacket {
                key: cookie_request_packet.key,
                payload: Some(authentication::sign(&auth_payload, &auth_secret)),
            })
            .await
            .expect("send cookie response failed");

        let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
            .read_packet()
            .await
            .expect("encryption request packet read failed");
        assert!(!encryption_request_packet.should_authenticate);

        let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
            .expect("public key deserialization failed");
        let enc_shared_secret = encrypt(&pub_key, shared_secret);
        let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
        client_stream
            .write_packet(login_in::EncryptionResponsePacket {
                shared_secret: enc_shared_secret,
                verify_token: enc_verify_token,
            })
            .await
            .expect("send encryption response failed");

        let (encryptor, decryptor) =
            authentication::create_ciphers(shared_secret).expect("create ciphers failed");
        let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));

        let login_success_packet: login_out::LoginSuccessPacket = client_stream
            .read_packet()
            .await
            .expect("login success packet read failed");
        assert_eq!(login_success_packet.user_name, user_name);
        assert_eq!(login_success_packet.user_id, user_id);

        client_stream
            .write_packet(login_in::LoginAcknowledgedPacket)
            .await
            .expect("send login acknowledged packet failed");

        // disconnect as no target configured
        let _disconnect_packet: conf_out::DisconnectPacket = client_stream
            .read_packet()
            .await
            .expect("disconnect packet read failed");

        // wait for the server to finish
        server.await.expect("server run failed");
    }
}
