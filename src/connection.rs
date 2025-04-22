use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::{Protocol, StatusSupplier};
use crate::adapter::target_selection::TargetSelector;
use crate::authentication;
use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use packets::{AsyncReadPacket, AsyncWritePacket, ReadPacket, ResourcePackResult, State, VarInt};
use packets::{Packet, WritePacket};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use std::io::{Cursor, ErrorKind};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{Instant, Interval};
use tracing::debug;
use uuid::Uuid;

use crate::metrics::{
    CONNECTION_DURATION, ConnectionDurationLabels, Guard, MOJANG_DURATION, MojangDurationLabels,
    RECEIVED_PACKETS, RESOURCEPACK_DURATION, ReceivedPackets, ResourcePackDurationLabels,
    SENT_PACKETS, SentPackets, TRANSFER_TARGETS, TransferTargetsLabels,
};
use packets::configuration::clientbound as conf_out;
use packets::configuration::serverbound as conf_in;
use packets::handshake::serverbound as hand_in;
use packets::login::clientbound as login_out;
use packets::login::serverbound as login_in;
use packets::status::clientbound as status_out;
use packets::status::serverbound as status_in;

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
            _ => return Err(Error::InvalidProtocol),
        }
    }};
}

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

    /// Keep-alive was not received.
    #[error("Missed keep-alive")]
    MissedKeepAlive,

    /// Some protocol error occurred.
    #[error("Some protocol error occurred")]
    InvalidProtocol,

    /// Some forced resource pack was not loaded.
    #[error("Some forced resourcepack was not loaded")]
    FailedResourcepack,

    /// No target was found for the user.
    #[error("No target was found for the user")]
    NoTargetFound,
}

impl Error {
    pub fn is_connection_closed(&self) -> bool {
        let err = match self {
            Error::Io(err) => err,
            Error::PacketError(err) => return err.is_connection_closed(),
            _ => return false,
        };

        err.kind() == ErrorKind::UnexpectedEof
            || err.kind() == ErrorKind::ConnectionReset
            || err.kind() == ErrorKind::ConnectionAborted
            || err.kind() == ErrorKind::BrokenPipe
    }

    pub fn as_label(&self) -> &'static str {
        if self.is_connection_closed() {
            return "closed";
        }

        match self {
            Error::PacketError(_) | Error::InvalidProtocol => "invalid-protocol",
            Error::MissedKeepAlive => "failed-keepalive",
            Error::FailedResourcepack => "failed-resourcepack",
            Error::NoTargetFound => "no-target",
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
}

#[derive(Debug)]
pub struct KeepAlive {
    pub packets: [u64; 2],
    pub last_sent: Instant,
    pub interval: Interval,
}

impl KeepAlive {
    pub fn replace(&mut self, from: u64, to: u64) -> bool {
        if self.packets[0] == from {
            self.packets[0] = to;
            true
        } else if self.packets[1] == from {
            self.packets[1] = to;
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
    /// The keep-alive config
    keep_alive: KeepAlive,
    /// The status supplier of the connection
    pub status_supplier: Arc<dyn StatusSupplier>,
    /// ...
    pub target_selector: Arc<dyn TargetSelector>,
    /// ...
    pub resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
    /// Auth cookie secret.
    pub auth_secret: Option<Vec<u8>>,
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite,
{
    pub fn new(
        stream: S,
        status_supplier: Arc<dyn StatusSupplier>,
        target_selector: Arc<dyn TargetSelector>,
        resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
        auth_secret: Option<Vec<u8>>,
    ) -> Connection<S> {
        // start ticker for keep-alive packets (use delay so that we don't miss any)
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        Self {
            stream: CipherStream::new(stream, None, None),
            keep_alive: KeepAlive {
                packets: [0; 2],
                last_sent: Instant::now(),
                interval,
            },
            status_supplier,
            target_selector,
            resourcepack_supplier,
            auth_secret,
        }
    }
}

impl<S> Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
{
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
                    self.keep_alive.last_sent = Instant::now();
                    let id = authentication::generate_keep_alive();
                    if !self.keep_alive.replace(0, id) {
                        self.send_packet(conf_out::DisconnectPacket {
                            reason: "Missed Keepalive".to_string(),
                        })
                        .await?;
                        return Err(Error::MissedKeepAlive);
                    }
                    let packet = conf_out::KeepAlivePacket { id };
                    self.send_packet(packet).await?;
                },
                // await the next packet in, reading the packet size (expect fast execution)
                maybe_length = self.stream.read_varint() => {
                    break maybe_length?;
                },
            }
        };

        RECEIVED_PACKETS.get_or_create(&ReceivedPackets {}).inc();

        // check the length of the packet for any following content
        if length == 0 || length > MAX_PACKET_LENGTH {
            debug!(
                length,
                "packet length should be between 0 and {MAX_PACKET_LENGTH}"
            );
            return Err(Error::PacketError(packets::Error::IllegalPacketLength));
        }

        // extract the encoded packet id
        let id = self.stream.read_varint().await?;

        // split a separate reader from the stream and read packet bytes (advancing stream)
        let mut buffer = vec![];
        (&mut self.stream)
            .take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .await?;
        let buf = Cursor::new(buffer);

        Ok((id, buf))
    }

    async fn send_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        SENT_PACKETS.get_or_create(&SentPackets {}).inc();
        Ok(self.stream.write_packet(packet).await?)
    }

    fn apply_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        debug!("enabling encryption");

        // get stream ciphers and wrap stream with cipher
        let (encryptor, decryptor) = authentication::create_ciphers(shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }

    pub async fn listen(&mut self, client_address: SocketAddr) -> Result<(), Error> {
        // hides connection closed errors
        match self.run_protocol(client_address).await {
            Err(err) => {
                if err.is_connection_closed() {
                    Ok(())
                } else {
                    Err(err)
                }
            }
            Ok(()) => Ok(()),
        }
    }

    async fn run_protocol(&mut self, client_address: SocketAddr) -> Result<(), Error> {
        let start = Instant::now();

        // handle handshake
        let handshake = match_packet! { self,
            packet = hand_in::HandshakePacket => packet,
        };

        // track metrics
        let variant = match handshake.next_state {
            State::Status => "status",
            State::Login => "login",
            State::Transfer => "transfer",
        };

        let _timer = Guard::on_drop(|| {
            CONNECTION_DURATION
                .get_or_create(&ConnectionDurationLabels {
                    variant,
                    protocol_version: handshake.protocol_version,
                })
                .observe(start.elapsed().as_secs_f64());
        });

        // handle status request
        if handshake.next_state == State::Status {
            let _ = match_packet! { self,
                packet = status_in::StatusRequestPacket => packet,
            };

            let status = self
                .status_supplier
                .get_status(
                    &client_address,
                    (&handshake.server_address, handshake.server_port),
                    handshake.protocol_version as Protocol,
                )
                .await?;

            self.send_packet(status_out::StatusResponsePacket {
                body: serde_json::to_string(&status)?,
            })
            .await?;

            let ping = match_packet! { self,
                packet = status_in::PingPacket => packet,
            };

            self.send_packet(status_out::PongPacket {
                payload: ping.payload,
            })
            .await?;

            return Ok(());
        }

        // handle login request
        let mut login_start = match_packet! { self,
            packet = login_in::LoginStartPacket => packet,
        };

        // in case of transfer, use the auth cookie
        let mut should_authenticate = true;
        'transfer: {
            if handshake.next_state == State::Transfer {
                if self.auth_secret.is_none() {
                    break 'transfer;
                }

                self.send_packet(login_out::CookieRequestPacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                })
                .await?;

                let cookie = match_packet! { self,
                    packet = login_in::CookieResponsePacket => packet,
                };

                let Some(message) = cookie.payload else {
                    break 'transfer;
                };

                let Some(secret) = &self.auth_secret else {
                    break 'transfer;
                };

                let (ok, message) = authentication::check_sign(&message, secret);
                if !ok {
                    break 'transfer;
                }

                let cookie = serde_json::from_slice::<AuthCookie>(message)?;
                let expires_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time error")
                    .as_secs()
                    + AUTH_COOKIE_EXPIRY_SECS;

                if cookie.client_addr.ip() != client_address.ip() || cookie.timestamp > expires_at {
                    break 'transfer;
                }

                should_authenticate = false;

                // update state by token
                login_start.user_name = cookie.user_name;
                login_start.user_id = cookie.user_id;
            }
        }

        // handle encryption
        let verify_token = authentication::generate_token()?;

        self.send_packet(login_out::EncryptionRequestPacket {
            server_id: "".to_owned(),
            public_key: authentication::ENCODED_PUB.clone(),
            verify_token,
            should_authenticate,
        })
        .await?;

        let encrypt = match_packet! { self,
            packet = login_in::EncryptionResponsePacket => packet,
        };

        // decrypt the shared secret and verify the token
        let shared_secret =
            authentication::decrypt(&authentication::KEY_PAIR.0, &encrypt.shared_secret)?;
        let decrypted_verify_token =
            authentication::decrypt(&authentication::KEY_PAIR.0, &encrypt.verify_token)?;

        // verify the token is correct
        authentication::verify_token(verify_token, &decrypted_verify_token)?;

        // handle authentication if not already authenticated by the token
        if should_authenticate {
            let start = Instant::now();
            let auth_response = authentication::authenticate_mojang(
                &login_start.user_name,
                &shared_secret,
                &authentication::ENCODED_PUB,
            )
            .await;

            let auth_response = match auth_response {
                Ok(resp) => {
                    MOJANG_DURATION
                        .get_or_create(&MojangDurationLabels { result: "success" })
                        .observe(start.elapsed().as_secs_f64());
                    resp
                }
                Err(err) => {
                    MOJANG_DURATION
                        .get_or_create(&MojangDurationLabels { result: "error" })
                        .observe(start.elapsed().as_secs_f64());
                    return Err(Error::from(err));
                }
            };

            // update state for actual use info
            login_start.user_name = auth_response.name;
            login_start.user_id = auth_response.id;
        }

        // enable encryption for the connection using the shared secret
        self.apply_encryption(&shared_secret)?;

        self.send_packet(login_out::LoginSuccessPacket {
            user_name: login_start.user_name.clone(),
            user_id: login_start.user_id,
        })
        .await?;

        let _ = match_packet! { self,
            packet = login_in::LoginAcknowledgedPacket => packet,
        };

        // write auth cookie
        'auth_cookie: {
            if should_authenticate {
                let Some(secret) = &self.auth_secret else {
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
                };

                let auth_payload = serde_json::to_vec(&cookie)?;
                self.send_packet(conf_out::StoreCookiePacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                    payload: authentication::sign(&auth_payload, secret),
                })
                .await?;
            }
        }

        // write resource packs
        let packs = self
            .resourcepack_supplier
            .get_resourcepacks(
                &client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login_start.user_name,
                &login_start.user_id,
            )
            .await?;
        let mut pack_ids: HashMap<Uuid, (bool, Instant)> = HashMap::with_capacity(packs.len());

        for pack in packs {
            let packet = conf_out::AddResourcePackPacket {
                uuid: pack.uuid,
                url: pack.url,
                hash: pack.hash,
                forced: pack.forced,
                prompt_message: pack.prompt_message,
            };
            self.send_packet(packet).await?;
            pack_ids.insert(pack.uuid, (pack.forced, Instant::now()));
        }

        // wait for resource packs to be accepted
        while !pack_ids.is_empty() {
            let packet = match_packet! { self, keep_alive,
                packet = conf_in::ResourcePackResponsePacket => packet,
                // handle keep alive packets
                packet = conf_in::KeepAlivePacket => {
                    if !self.keep_alive.replace(packet.id, 0) {
                        debug!(id = packet.id, "keep alive packet id unknown");
                    }
                    continue;
                },
                // ignore unsupported packets but don't throw an error
                _ = conf_in::ClientInformationPacket => continue,
                _ = conf_in::PluginMessagePacket => continue,
            };

            // check the state for any final state in the resource pack loading process
            let success = match packet.result {
                ResourcePackResult::Success => true,
                ResourcePackResult::Declined
                | ResourcePackResult::DownloadFailed
                | ResourcePackResult::InvalidUrl
                | ResourcePackResult::ReloadFailed
                | ResourcePackResult::Discorded => false,
                // pending state, keep waiting
                _ => continue,
            };

            // pop pack from the list (ignoring unknown pack ids)
            let Some((forced, time)) = pack_ids.remove(&packet.uuid) else {
                continue;
            };

            if success {
                RESOURCEPACK_DURATION
                    .get_or_create(&ResourcePackDurationLabels {
                        uuid: packet.uuid.to_string(),
                    })
                    .observe(time.elapsed().as_secs_f64());
            }

            // handle pack forced
            if forced && !success {
                // TODO write actual reason
                self.send_packet(conf_out::DisconnectPacket {
                    reason: "".to_string(),
                })
                .await?;
                return Err(Error::FailedResourcepack);
            }
        }

        // transfer
        let target = self
            .target_selector
            .select(
                &client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login_start.user_name,
                &login_start.user_id,
            )
            .await?;

        TRANSFER_TARGETS
            .get_or_create(&TransferTargetsLabels {
                target: target.clone().map(|target| target.to_string()),
            })
            .inc();

        // disconnect if not target found
        let Some(target) = target else {
            // TODO write actual message
            self.send_packet(conf_out::DisconnectPacket {
                reason: "".to_string(),
            })
            .await?;
            return Err(Error::NoTargetFound);
        };

        // create a new transfer packet and send it
        let transfer = conf_out::TransferPacket {
            host: target.ip().to_string(),
            port: target.port(),
        };
        self.send_packet(transfer).await?;

        Ok(())
    }
}
