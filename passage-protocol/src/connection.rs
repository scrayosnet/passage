use crate::cookie::{
    AUTH_COOKIE_EXPIRY_SECS, AUTH_COOKIE_KEY, AuthCookie, SESSION_COOKIE_KEY, SessionCookie, sign,
    verify,
};
use crate::crypto::stream::{Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream, create_ciphers};
pub(crate) use crate::error::Error;
use crate::localization::Localization;
use crate::mojang::Mojang;
use crate::{crypto, metrics};
use passage_adapters::{
    Protocol, discovery::DiscoveryAdapter, status::StatusAdapter, strategy::StrategyAdapter,
};
use passage_packets::configuration::clientbound as conf_out;
use passage_packets::configuration::serverbound as conf_in;
use passage_packets::handshake::serverbound as hand_in;
use passage_packets::login::clientbound as login_out;
use passage_packets::login::serverbound as login_in;
use passage_packets::status::clientbound as status_out;
use passage_packets::status::serverbound as status_in;
use passage_packets::{
    AsyncReadPacket, AsyncWritePacket, INITIAL_BUFFER_SIZE, ReadPacket, State, VarInt,
};
use passage_packets::{Packet, WritePacket};
use std::fmt::Debug;
use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Instant, Interval};
use tracing::{Instrument, debug, error, field, info, instrument};
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
pub const DEFAULT_MAX_PACKET_LENGTH: VarInt = 10_000;

/// The interval in seconds at which keep-alive packets are sent. Has to be between 15 and 20 seconds,
/// such that at most one keep-alive packet is in transit at any point.
pub const KEEP_ALIVE_INTERVAL: u64 = 16;

pub struct Connection<S, Disc, Stat, Stra, Api> {
    stream: CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec>,
    buffer: Vec<u8>,

    // adapters
    status_adapter: Arc<Stat>,
    discovery_adapter: Arc<Disc>,
    strategy_adapter: Arc<Stra>,
    mojang: Arc<Api>,
    localization: Arc<Localization>,

    // config and internal state
    keep_alive_id: Option<u64>,
    keep_alive_interval: Interval,
    auth_secret: Option<Vec<u8>>,
    max_packet_length: VarInt,

    // client information
    client_address: SocketAddr,
    client_locale: String,
}

impl<S, Disc, Stat, Stra, Api> Connection<S, Disc, Stat, Stra, Api>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Disc: DiscoveryAdapter,
    Stat: StatusAdapter,
    Stra: StrategyAdapter,
    Api: Mojang,
{
    pub fn new(
        stream: S,
        status_adapter: Arc<Stat>,
        discovery_adapter: Arc<Disc>,
        strategy_adapter: Arc<Stra>,
        mojang: Arc<Api>,
        localization: Arc<Localization>,
        auth_secret: Option<Vec<u8>>,
        client_address: SocketAddr,
    ) -> Self {
        // start ticker for keep-alive packets, it has to be sent at least every 20 seconds.
        // Then, the client has 15 seconds to respond with a keep-alive packet. We ensure that only
        // one keep-alive is in transit at any time and has to be answered before the next is sent.
        let mut interval = tokio::time::interval(Duration::from_secs(KEEP_ALIVE_INTERVAL));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // use the default locale as the client locale until the real locale is known
        let client_locale = localization.default_locale.clone();

        Self {
            stream: CipherStream::from_stream(stream),
            buffer: Vec::with_capacity(INITIAL_BUFFER_SIZE),
            // adapters
            status_adapter,
            discovery_adapter,
            strategy_adapter,
            mojang,
            localization,
            // config and internal state
            keep_alive_id: None,
            keep_alive_interval: interval,
            auth_secret,
            max_packet_length: DEFAULT_MAX_PACKET_LENGTH,
            // client information
            client_address,
            client_locale,
        }
    }

    pub fn with_max_packet_length(mut self, max_packet_length: VarInt) -> Self {
        self.max_packet_length = max_packet_length;
        self
    }

    #[instrument(skip_all, fields(packet_length = field::Empty, packet_id = field::Empty))]
    async fn receive_packet(
        &mut self,
        keep_alive: bool,
    ) -> Result<(VarInt, Cursor<Vec<u8>>), Error> {
        // wait for the next packet, send keep-alive packets as necessary
        let length = loop {
            tokio::select! {
                // use biased selection such that branches are checked in order
                biased;
                // await the next timer tick for keep-alive
                _ = self.keep_alive_interval.tick() => {
                    if !keep_alive { continue; }
                    debug!("checking that keep-alive packet was received");
                    if self.keep_alive_id.is_some() {
                        let reason = self.localization.localize(&self.client_locale, "disconnect_timeout", &[]);
                        self.send_packet(conf_out::DisconnectPacket { reason }).await?;
                        return Err(Error::MissedKeepAlive);
                    }
                    debug!("sending next keep-alive packet");
                    let id = crypto::generate_keep_alive();
                    self.keep_alive_id = Some(id);
                    let packet = conf_out::KeepAlivePacket { id };
                    self.send_packet(packet).await?;
                },
                // await the next packet in, reading the packet size (expect fast execution)
                maybe_length = self.stream.read_varint().instrument(tracing::info_span!("read_packet_length", otel.kind = "server")) => {
                    break maybe_length?;
                },
            }
        };

        // check the length of the packet for any following content
        if length <= 0 || length > self.max_packet_length {
            debug!(
                length,
                "packet length should be between 0 and {}", self.max_packet_length
            );
            return Err(passage_packets::Error::IllegalPacketLength.into());
        }

        // track metrics
        let packet_size = u64::try_from(length).expect("length is always positive");
        metrics::packet_size::record_serverbound(packet_size);
        tracing::Span::current().record("packet_length", packet_size);

        // extract the encoded packet id
        let id = self
            .stream
            .read_varint()
            .instrument(tracing::info_span!("read_packet_id", otel.kind = "server"))
            .await?;
        tracing::Span::current().record("packet_id", id);

        // split a separate reader from the stream and read packet bytes (advancing stream)
        let mut buffer = vec![];
        (&mut self.stream)
            .take(length as u64 - 1)
            .read_to_end(&mut buffer)
            .instrument(tracing::info_span!(
                "read_packet_bytes",
                otel.kind = "server"
            ))
            .await?;
        let buf = Cursor::new(buffer);

        Ok((id, buf))
    }

    #[instrument(skip_all)]
    async fn send_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        // write the packets id and the respective packets content
        self.buffer.clear();
        self.buffer.write_varint(T::ID as VarInt).await?;
        packet.write_to_buffer(&mut self.buffer).await?;

        // prepare a final buffer (leaving max 2 bytes for varint as packets never get that big)
        let packet_len = self.buffer.len();
        // TODO reuse buffer here or write twice!
        let mut final_buffer = Vec::with_capacity(packet_len + 2);
        final_buffer.write_varint(packet_len as VarInt).await?;
        final_buffer.extend_from_slice(&self.buffer);

        // send the final buffer into the stream
        self.stream
            .write_all(&final_buffer)
            .instrument(tracing::info_span!("write_packet", otel.kind = "server"))
            .await?;

        // track metrics
        let packet_size = u64::try_from(final_buffer.len()).expect("usize always fits into u64");
        metrics::packet_size::record_clientbound(packet_size);

        Ok(())
    }

    fn handle_keep_alive(&mut self, id: u64) {
        if self.keep_alive_id == Some(id) {
            self.keep_alive_id = None;
        } else {
            debug!(id = id, "keep alive packet id unknown");
        }
    }

    // TODO check whether this may result in partially written packets?
    /** Endlessly receives and sends keep-alive packets. It should be used with a `tokio::select!` */
    async fn keep_alive<T>(&mut self) -> Result<T, Error> {
        loop {
            match_packet! { self, keep_alive,
                // handle keep alive packets
                packet = conf_in::KeepAlivePacket => {
                    self.handle_keep_alive(packet.id);
                    continue
                },
                // ignore unsupported packets but don't throw an error
                _ = conf_in::ClientInformationPacket => continue,
                _ = conf_in::PluginMessagePacket => continue,
                _ = conf_in::ResourcePackResponsePacket => continue,
                _ = conf_in::CookieResponsePacket => continue,
            }
        }
    }

    #[instrument(skip_all)]
    fn apply_encryption(&mut self, shared_secret: &[u8]) -> Result<(), Error> {
        debug!("enabling encryption");

        // get stream ciphers and wrap stream with cipher
        let (encryptor, decryptor) = create_ciphers(shared_secret)?;
        self.stream.set_encryption(Some(encryptor), Some(decryptor));

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn listen(&mut self) -> Result<(), Error> {
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
                .status_adapter
                .status(
                    &self.client_address,
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
        let session_packet = match_packet! { self,
            packet = login_in::CookieResponsePacket => packet,
        };
        let session_cookie = session_packet.decode::<SessionCookie>()?;

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

                let (ok, message) = verify(&signed, secret);
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

                if cookie.client_addr.ip() != self.client_address.ip() || expires_at < now {
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
        let verify_token = crypto::generate_token()?;

        debug!("sending encryption request packet");
        self.send_packet(login_out::EncryptionRequestPacket {
            server_id: String::new(),
            public_key: crypto::ENCODED_PUB.clone(),
            verify_token,
            should_authenticate,
        })
        .await?;

        debug!("awaiting encryption response packet");
        let encrypt = match_packet! { self,
            packet = login_in::EncryptionResponsePacket => packet,
        };

        // decrypt the shared secret and verify the token
        let shared_secret = crypto::decrypt(&crypto::KEY_PAIR.0, &encrypt.shared_secret)?;
        let decrypted_verify_token = crypto::decrypt(&crypto::KEY_PAIR.0, &encrypt.verify_token)?;

        // verify the token is correct
        debug!("verifying verify token");
        if !crypto::verify_token(verify_token, &decrypted_verify_token) {
            return Err(Error::InvalidVerifyToken);
        }

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
                    &crypto::ENCODED_PUB,
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
                    self.handle_keep_alive(packet.id);
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

        // wait for target task to finish and send keep alive packets
        // technically, this could be done a lot earlier (maybe even in a separate threat), however
        // in the future we might want to consider the client information when selecting a target
        debug!("discovering targets");
        let discovery_adapter = self.discovery_adapter.clone();
        let targets = tokio::select! {
            result = self.keep_alive() => result?,
            maybe_targets = discovery_adapter.discover() => maybe_targets?,
        };

        debug!("selecting target");
        let strategy_adapter = self.strategy_adapter.clone();
        let client_address = self.client_address;
        let target = tokio::select! {
            result = self.keep_alive() => result?,
            maybe_target = strategy_adapter.select(
                &client_address,
                (&handshake.server_address, handshake.server_port),
                handshake.protocol_version as Protocol,
                &login_start.user_name,
                &login_start.user_id,
                targets,
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
                    client_addr: self.client_address,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("time error")
                        .as_secs(),
                    user_name: login_start.user_name.clone(),
                    user_id: login_start.user_id,
                    target: Some(target.identifier.clone()),
                    profile_properties,
                    extra: Default::default(),
                };

                let auth_payload = serde_json::to_vec(&cookie)?;
                debug!("sending auth cookie packet");
                self.send_packet(conf_out::StoreCookiePacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                    payload: sign(&auth_payload, secret),
                })
                .await?;
            }
        }

        // set session id if not exist (does not override the session fields)
        if session_cookie.is_none() {
            debug!("sending session cookie packet");
            self.send_packet(conf_out::StoreCookiePacket {
                key: SESSION_COOKIE_KEY.to_string(),
                payload: serde_json::to_vec(&SessionCookie {
                    id: Uuid::new_v4(),
                    server_address: handshake.server_address.clone(),
                    server_port: handshake.server_port,
                })?,
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
