use crate::adapter::status::{Protocol, StatusSupplier};
use crate::adapter::target_selection::TargetSelector;
use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use crate::config::Localization;
use crate::mojang::{Mojang, ProfileProperty};
use crate::{authentication, metrics};
use packets::configuration::clientbound as conf_out;
use packets::configuration::serverbound as conf_in;
use packets::handshake::serverbound as hand_in;
use packets::login::clientbound as login_out;
use packets::login::serverbound as login_in;
use packets::status::clientbound as status_out;
use packets::status::serverbound as status_in;
use packets::{AsyncReadPacket, AsyncWritePacket, ReadPacket, State, VarInt};
use packets::{Packet, WritePacket};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::io::{Cursor, ErrorKind};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Instant, Interval};
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

#[macro_export]
macro_rules! match_packet {
    // macro variant without sending keep-alive packets
    { $con:expr, $($packet:pat = $packet_type:ty => $handler:expr,)* } => {
        match_packet! { $con, false, $($packet = $packet_type => $handler,)* }
    };
    // macro variant with sending keep-alive packets
    { $con:expr, keep_alive, $($packet:pat = $packet_type:ty => $handler:expr,)* } => {
        match_packet! { $con, true, $($packet = $packet_type => $handler,)* }
    };
    // general macro implementation with boolean for sending keep-alive packets
    {$con:expr, $keep_alive:expr, $($packet:pat = $packet_type:ty => $handler:expr,)* } => {{
        let (id, mut buf) = $con.receive_packet($keep_alive).await?;
        match id {
            $(
                <$packet_type>::ID => {
                    let $packet = <$packet_type>::read_from_buffer(&mut buf).await?;
                    $handler
                },
            )*
            _ => return Err(Error::UnexpectedPacketId(id)),
        }
    }};
}

/// The max packet length in bytes. Larger packets are rejected.
const MAX_PACKET_LENGTH: VarInt = 10_000;

/// The auth cookie key.
pub const AUTH_COOKIE_KEY: &str = "passage:authentication";

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

/// The default expiry of the auth cookie (6 hours).
pub const AUTH_COOKIE_EXPIRY_SECS: u64 = 6 * 60 * 60;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An unrecognized io error. All expected io errors are
    #[error("unexpected io error: {0}")]
    InternalIo(std::io::Error),

    /// The JSON version of a packet content could not be encoded.
    #[error("invalid struct for JSON (encoding problem)")]
    Json(#[from] serde_json::Error),

    /// Some crypto/authentication request failed.
    #[error("could not encrypt connection: {0}")]
    CryptographyFailed(#[from] authentication::Error),

    #[error("authentication request failed: {0}")]
    AuthRequestFailed(#[from] reqwest::Error),

    /// Keep-alive was not received.
    #[error("Missed keep-alive")]
    MissedKeepAlive,

    /// No target was found for the user.
    #[error("No target was found for the user")]
    NoTargetFound,

    /// The connection was closed, presumably by the client.
    #[error("The connection was closed (by the client)")]
    ConnectionClosed(std::io::Error),

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
    #[error("unexpected packet id received {0}")]
    UnexpectedPacketId(VarInt),

    /// The JSON response of a packet is incorrectly encoded (not UTF-8).
    #[error("invalid response body (invalid encoding)")]
    InvalidEncoding,

    /// Some array conversion failed.
    #[error("could not convert into array")]
    ArrayConversionFailed,

    /// Some fastnbt error.
    #[error("failed to parse nbt: {0}")]
    Nbt(#[from] packets::fastnbt::error::Error),

    /// An error occurred during the invocation or communication of an adapter.
    #[error("failed to invoke adapter: {0}")]
    AdapterError(#[from] crate::adapter::Error),
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        match value.kind() {
            ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::NetworkDown
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::WriteZero
            | ErrorKind::UnexpectedEof => Error::ConnectionClosed(value),
            _ => Error::InternalIo(value),
        }
    }
}

impl From<packets::Error> for Error {
    fn from(value: packets::Error) -> Self {
        match value {
            packets::Error::Io(err) => err.into(),
            packets::Error::IllegalPacketLength => Error::IllegalPacketLength,
            packets::Error::IllegalEnumValue { kind, value } => {
                Error::IllegalEnumValue { kind, value }
            }
            packets::Error::IllegalPacketId { actual, .. } => Error::UnexpectedPacketId(actual),
            packets::Error::InvalidEncoding => Error::InvalidEncoding,
            packets::Error::ArrayConversionFailed => Error::ArrayConversionFailed,
            packets::Error::Json(err) => Error::Json(err),
            packets::Error::Nbt(err) => Error::Nbt(err),
        }
    }
}

impl Error {
    pub fn as_label(&self) -> &'static str {
        match self {
            Error::MissedKeepAlive => "missed-keep-alive",
            Error::NoTargetFound => "no-target-found",
            Error::ConnectionClosed(_) => "connection-closed",
            Error::IllegalPacketLength
            | Error::IllegalEnumValue { .. }
            | Error::UnexpectedPacketId { .. }
            | Error::InvalidEncoding => "protocol-error",
            _ => "internal-error",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthCookie {
    pub timestamp: u64,
    pub client_addr: SocketAddr,
    pub user_name: String,
    pub user_id: Uuid,
    pub target: Option<String>,
    pub profile_properties: Vec<ProfileProperty>,
}

#[derive(Debug)]
pub struct KeepAlive<const SIZE: usize> {
    pub packets: [u64; SIZE],
    pub last_sent: Instant,
    pub interval: Interval,
}

impl<const SIZE: usize> KeepAlive<SIZE> {
    pub fn replace(&mut self, from: u64, to: u64) -> bool {
        for entry in &mut self.packets {
            if *entry == from {
                *entry = to;
                return true;
            }
        }
        false
    }
}

/// ...
pub struct Connection<S> {
    /// The connection reader
    stream: CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec>,
    /// The keep-alive config
    keep_alive: KeepAlive<2>,
    /// The status supplier of the connection
    pub status_supplier: Arc<dyn StatusSupplier>,
    /// ...
    pub target_selector: Arc<dyn TargetSelector>,
    /// ...
    pub mojang: Arc<dyn Mojang>,
    /// Auth cookie secret.
    pub auth_secret: Option<Vec<u8>>,
    /// The currently registered client locale. It falls back to the globally configured default.
    pub client_locale: String,
    /// The localization handler.
    pub localization: Arc<Localization>,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite,
{
    pub fn new(
        stream: S,
        status_supplier: Arc<dyn StatusSupplier>,
        target_selector: Arc<dyn TargetSelector>,
        mojang: Arc<dyn Mojang>,
        localization: Arc<Localization>,
        auth_secret: Option<Vec<u8>>,
    ) -> Connection<S> {
        // start ticker for keep-alive packets (use delay so that we don't miss any)
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        Self {
            stream: CipherStream::new(stream, None, None),
            keep_alive: KeepAlive {
                // array size is based on interval duration
                packets: [0; 2],
                last_sent: Instant::now(),
                interval,
            },
            status_supplier,
            target_selector,
            mojang,
            auth_secret,
            client_locale: localization.default_locale.clone(),
            localization,
        }
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
{
    #[instrument(skip_all)]
    async fn receive_packet(
        &mut self,
        keep_alive: bool,
    ) -> Result<(VarInt, Cursor<Vec<u8>>), Error> {
        // start ticker for keep-alive packets (use delay so that we don't miss any)
        let duration = Duration::from_secs(10);
        let mut interval = tokio::time::interval_at(self.keep_alive.last_sent + duration, duration);

        // wait for the next packet, send keep-alive packets as necessary
        let length = loop {
            tokio::select! {
                // use biased selection such that branches are checked in order
                biased;
                // await the next timer tick for keep-alive
                _ = interval.tick() => {
                    if !keep_alive { continue; }
                    debug!("checking that keep-alive packet was received");
                    self.keep_alive.last_sent = Instant::now();
                    let id = authentication::generate_keep_alive();
                    if !self.keep_alive.replace(0, id) {
                        let reason = self.localization.localize(&self.client_locale, "disconnect_timeout", &[]);
                        self.send_packet(conf_out::DisconnectPacket { reason }).await?;
                        return Err(Error::MissedKeepAlive);
                    }
                    debug!("sending next keep-alive packet");
                    let packet = conf_out::KeepAlivePacket { id };
                    self.send_packet(packet).await?;
                },
                // await the next packet in, reading the packet size (expect fast execution)
                maybe_length = self.stream.read_varint() => {
                    break maybe_length?;
                },
            }
        };

        // check the length of the packet for any following content
        if length <= 0 || length > MAX_PACKET_LENGTH {
            debug!(
                length,
                "packet length should be between 0 and {MAX_PACKET_LENGTH}"
            );
            return Err(packets::Error::IllegalPacketLength.into());
        }

        // track metrics
        let packet_size = u64::try_from(length).expect("length is always positive");
        metrics::packet_size::record_serverbound(packet_size);
        tracing::Span::current().record("packet_length", packet_size);

        // extract the encoded packet id
        let id = self.stream.read_varint().await?;
        tracing::Span::current().record("packet_id", id);

        // split a separate reader from the stream and read packet bytes (advancing stream)
        let mut buffer = vec![];
        (&mut self.stream)
            .take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .await?;
        let buf = Cursor::new(buffer);

        Ok((id, buf))
    }

    #[instrument(skip_all)]
    async fn send_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        // write the packet to the stream
        let bytes_written = self.stream.write_packet(packet).await?;

        // track metrics
        let packet_size = u64::try_from(bytes_written).expect("usize always fits into u64");
        metrics::packet_size::record_clientbound(packet_size);

        Ok(())
    }

    #[instrument(skip_all)]
    fn apply_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        debug!("enabling encryption");

        // get stream ciphers and wrap stream with cipher
        let (encryptor, decryptor) = authentication::create_ciphers(shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn listen(&mut self, client_address: SocketAddr) -> Result<(), Error> {
        // handle handshake
        debug!("awaiting handshake packet");
        let handshake = match_packet! { self,
            packet = hand_in::HandshakePacket => packet,
        };

        // track metrics
        let variant = match handshake.next_state {
            State::Status => "status",
            State::Login => "login",
            State::Transfer => "transfer",
        };
        debug!(next_state = variant, "received handshake packet");

        // handle status request
        if handshake.next_state == State::Status {
            debug!("awaiting status request packet");
            let _ = match_packet! { self,
                packet = status_in::StatusRequestPacket => packet,
            };

            debug!("getting status from supplier");
            let status = self
                .status_supplier
                .get_status(
                    &client_address,
                    (&handshake.server_address, handshake.server_port),
                    handshake.protocol_version as Protocol,
                )
                .await?;

            debug!("sending status response packet");
            self.send_packet(status_out::StatusResponsePacket {
                body: serde_json::to_string(&status)?,
            })
            .await?;

            debug!("awaiting ping packet");
            let ping = match_packet! { self,
                packet = status_in::PingPacket => packet,
            };

            debug!("sending pong packet");
            self.send_packet(status_out::PongPacket {
                payload: ping.payload,
            })
            .await?;

            return Ok(());
        }

        // handle login request
        debug!("awaiting login start packet");
        let mut login_start = match_packet! { self,
            packet = login_in::LoginStartPacket => packet,
        };

        info!(
            user_name = login_start.user_name,
            user_id = login_start.user_id.to_string(),
            "handling login"
        );

        // check session
        debug!("sending session cookie request packet");
        self.send_packet(login_out::CookieRequestPacket {
            key: SESSION_COOKIE_KEY.to_string(),
        })
        .await?;

        debug!("awaiting session cookie response packet");
        let session_cookie = match_packet! { self,
            packet = login_in::CookieResponsePacket => packet,
        };
        let session_id = session_cookie.payload;

        // in case of transfer, use the auth cookie
        let mut should_authenticate = true;
        let mut profile_properties = vec![];
        'transfer: {
            if handshake.next_state == State::Transfer {
                if self.auth_secret.is_none() {
                    debug!("no auth secret configured, skipping auth cookie");
                    break 'transfer;
                }

                debug!("sending auth cookie request packet");
                self.send_packet(login_out::CookieRequestPacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                })
                .await?;

                debug!("awaiting auth cookie response packet");
                let cookie = match_packet! { self,
                    packet = login_in::CookieResponsePacket => packet,
                };

                let Some(signed) = cookie.payload else {
                    debug!("no auth cookie received, skipping auth cookie");
                    break 'transfer;
                };

                let Some(secret) = &self.auth_secret else {
                    debug!("no auth secret configured, skipping auth cookie");
                    break 'transfer;
                };

                let (ok, message) = authentication::verify(&signed, secret);
                if !ok {
                    debug!("invalid auth cookie signature received, skipping auth cookie");
                    break 'transfer;
                }

                let cookie = serde_json::from_slice::<AuthCookie>(message)?;
                let expires_at = cookie.timestamp + AUTH_COOKIE_EXPIRY_SECS;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time error")
                    .as_secs();

                if cookie.client_addr.ip() != client_address.ip() || expires_at < now {
                    debug!("invalid auth cookie payload received, skipping auth cookie");
                    break 'transfer;
                }

                should_authenticate = false;

                // update state by token
                login_start.user_name = cookie.user_name;
                login_start.user_id = cookie.user_id;
                profile_properties = cookie.profile_properties;
            }
        }

        // handle encryption
        let verify_token = authentication::generate_token()?;

        debug!("sending encryption request packet");
        self.send_packet(login_out::EncryptionRequestPacket {
            server_id: String::new(),
            public_key: authentication::ENCODED_PUB.clone(),
            verify_token,
            should_authenticate,
        })
        .await?;

        debug!("awaiting encryption response packet");
        let encrypt = match_packet! { self,
            packet = login_in::EncryptionResponsePacket => packet,
        };

        // decrypt the shared secret and verify the token
        let shared_secret =
            authentication::decrypt(&authentication::KEY_PAIR.0, &encrypt.shared_secret)?;
        let decrypted_verify_token =
            authentication::decrypt(&authentication::KEY_PAIR.0, &encrypt.verify_token)?;

        // verify the token is correct
        debug!("verifying verify token");
        authentication::verify_token(verify_token, &decrypted_verify_token)?;

        // handle authentication if not already authenticated by the token
        if should_authenticate {
            debug!("authenticating with mojang");
            let request_start = Instant::now();
            let auth_response = self
                .mojang
                .authenticate(
                    &login_start.user_name,
                    &shared_secret,
                    "",
                    &authentication::ENCODED_PUB,
                )
                .await
                .inspect_err(|err| {
                    // track request failed
                    error!(err = %err, "mojang request failed");
                    metrics::mojang_request_duration::record(request_start, "failed");
                })?;
            metrics::mojang_request_duration::record(request_start, "success");

            // update state for actual use info
            login_start.user_name = auth_response.name;
            login_start.user_id = auth_response.id;
            profile_properties = auth_response.properties;
        }

        // enable encryption for the connection using the shared secret
        self.apply_encryption(&shared_secret)?;

        debug!("sending login success packet");
        self.send_packet(login_out::LoginSuccessPacket {
            user_name: login_start.user_name.clone(),
            user_id: login_start.user_id,
        })
        .await?;

        debug!("awaiting login acknowledged packet");
        let _ = match_packet! { self,
            packet = login_in::LoginAcknowledgedPacket => packet,
        };

        // wait for a client information packet
        debug!("awaiting client information packet");
        let client_info = loop {
            match_packet! { self, keep_alive,
                // handle keep alive packets
                packet = conf_in::KeepAlivePacket => {
                    if !self.keep_alive.replace(packet.id, 0) {
                        debug!(id = packet.id, "keep alive packet id unknown");
                    }
                    continue;
                },
                // wait for a client information packet
                packet = conf_in::ClientInformationPacket => break packet,
                // ignore unsupported packets but don't throw an error
                _ = conf_in::PluginMessagePacket => continue,
                _ = conf_in::ResourcePackResponsePacket => continue,
                _ = conf_in::CookieResponsePacket => continue,
            }
        };

        // track client metrics
        metrics::client_locale::inc(client_info.locale.clone());
        metrics::client_view_distance::record(
            u64::try_from(client_info.view_distance).unwrap_or(0u64),
        );

        // create a future that checks keep alive packets
        let target_selector = self.target_selector.clone();
        let sender = async {
            loop {
                match_packet! { self, keep_alive,
                    // handle keep alive packets
                    packet = conf_in::KeepAlivePacket => {
                        if !self.keep_alive.replace(packet.id, 0) {
                            debug!(id = packet.id, "keep alive packet id unknown");
                        }
                        continue
                    },
                    // ignore unsupported packets but don't throw an error
                    _ = conf_in::ClientInformationPacket => continue,
                    _ = conf_in::PluginMessagePacket => continue,
                    _ = conf_in::ResourcePackResponsePacket => continue,
                    _ = conf_in::CookieResponsePacket => continue,
                }
            }
        };

        // wait for target task to finish and send keep alive packets
        // technically, this could be done a lot earlier (maybe even in a separate threat), however
        // in the future we might want to consider the client information when selecting a target
        debug!("getting target from supplier");
        let target = tokio::select! {
            result = sender => result?,
            maybe_target = target_selector.select(
                &client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login_start.user_name,
                &login_start.user_id,
            ) => maybe_target?,
        };

        // disconnect if not target found
        let Some(target) = target else {
            debug!("no transfer target found");
            let reason =
                self.localization
                    .localize(&self.client_locale, "disconnect_no_target", &[]);
            debug!("sending disconnect packet");
            self.send_packet(conf_out::DisconnectPacket { reason })
                .await?;
            return Err(Error::NoTargetFound);
        };

        // write auth cookie
        'auth_cookie: {
            if should_authenticate {
                debug!("writing auth cookie");

                let Some(secret) = &self.auth_secret else {
                    debug!("no auth secret configured, skipping writing auth cookie");
                    break 'auth_cookie;
                };

                let cookie = AuthCookie {
                    client_addr: client_address,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("time error")
                        .as_secs(),
                    user_name: login_start.user_name.clone(),
                    user_id: login_start.user_id,
                    target: Some(target.identifier.clone()),
                    profile_properties,
                };

                let auth_payload = serde_json::to_vec(&cookie)?;
                debug!("sending auth cookie packet");
                self.send_packet(conf_out::StoreCookiePacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                    payload: authentication::sign(&auth_payload, secret),
                })
                .await?;
            }
        }

        // set session id if not exist
        if session_id.is_none() {
            debug!("sending session cookie packet");
            self.send_packet(conf_out::StoreCookiePacket {
                key: SESSION_COOKIE_KEY.to_string(),
                payload: Uuid::new_v4().into_bytes().to_vec(),
            })
            .await?;
        }

        // create a new transfer packet and send it
        let transfer = conf_out::TransferPacket {
            host: target.address.ip().to_string(),
            port: target.address.port(),
        };
        debug!("sending transfer packet");
        self.send_packet(transfer).await?;

        Ok(())
    }
}
