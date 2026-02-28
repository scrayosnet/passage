use crate::crypto::cookie::{
    sign, verify, AuthCookie, SessionCookie, AUTH_COOKIE_KEY, SESSION_COOKIE_KEY,
};
use crate::crypto::stream::{create_ciphers, Aes128Cfb8Dec, Aes128Cfb8Enc, CipherStream};
use crate::error::Error;
use crate::protocol::config::Config;
use crate::{crypto, metrics};
use opentelemetry::trace::TraceContextExt;
use passage_adapters::{
    Adapters, AuthenticationAdapter, DiscoveryAdapter, FilterAdapter, LocalizationAdapter,
    Protocol, StatusAdapter, StrategyAdapter,
};
use passage_packets::configuration::clientbound as conf_out;
use passage_packets::configuration::serverbound as conf_in;
use passage_packets::handshake::serverbound as hand_in;
use passage_packets::login::clientbound as login_out;
use passage_packets::login::serverbound as login_in;
use passage_packets::status::clientbound as status_out;
use passage_packets::status::serverbound as status_in;
use passage_packets::{
    AsyncReadPacket, AsyncWritePacket, ReadPacket, State, VarInt, INITIAL_BUFFER_SIZE,
};
use passage_packets::{Packet, WritePacket};
use std::fmt::Debug;
use std::io::Cursor;
use std::net::SocketAddr;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

#[macro_export]
macro_rules! match_packet {
    {
        $packet:expr;
        $($packet_bind:pat = $packet_type:ty => $packet_handler:expr,)*
    } => {{
        let (id, mut buf) = $packet;
        match id {
            $(
                <$packet_type>::ID => {
                    let $packet_bind = <$packet_type>::read_from_buffer(&mut buf).await?;
                    $packet_handler
                },
            )*
            _ => return Err(Error::UnexpectedPacketId(id)),
        }
    }};
}

/// The interval in seconds at which keep-alive packets are sent. Has to be between 15 and 20 seconds,
/// such that at most one keep-alive packet is in transit at any point.
pub const KEEP_ALIVE_INTERVAL: u64 = 16;

pub struct Connection<S, Stat, Disc, Filt, Stra, Auth, Loca> {
    stream: CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec>,
    buffer: Vec<u8>,

    // configuration
    adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,
    config: Config,
    client_address: SocketAddr,

    // runtime state
    keep_alive_id: Option<u64>,
    client_locale: Option<String>,
}

impl<S, Stat, Disc, Filt, Stra, Auth, Loca> Connection<S, Stat, Disc, Filt, Stra, Auth, Loca>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    Stat: StatusAdapter + 'static,
    Disc: DiscoveryAdapter + 'static,
    Filt: FilterAdapter + 'static,
    Stra: StrategyAdapter + 'static,
    Auth: AuthenticationAdapter + 'static,
    Loca: LocalizationAdapter + 'static,
{
    pub fn new(
        stream: S,
        adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,
        config: Config,
        client_address: SocketAddr,
    ) -> Self {
        Self {
            stream: CipherStream::from_stream(stream),
            buffer: Vec::with_capacity(INITIAL_BUFFER_SIZE),
            adapters,
            config,
            client_address,
            keep_alive_id: None,
            client_locale: None,
        }
    }

    #[instrument(skip_all)]
    async fn next_packet(&mut self) -> Result<VarInt, Error> {
        let length = self
            .stream
            .read_varint()
            .instrument(tracing::info_span!(
                "read_packet_length",
                otel.kind = "server"
            ))
            .await?;
        Ok(length)
    }

    #[instrument(skip(self))]
    async fn read_packet(&mut self, length: VarInt) -> Result<(VarInt, Cursor<Vec<u8>>), Error> {
        // check the length of the packet for any following content
        if length <= 0 || length > self.config.max_packet_length {
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

    #[instrument(skip(self))]
    async fn read_next_packet(&mut self) -> Result<(VarInt, Cursor<Vec<u8>>), Error> {
        let length = self.next_packet().await?;
        self.read_packet(length).await
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

    #[instrument(skip_all)]
    async fn send_keep_alive(&mut self) -> Result<(), Error> {
        debug!("checking that keep-alive packet was received");
        if self.keep_alive_id.is_some() {
            let reason = self
                .adapters
                .localize(self.client_locale.as_deref(), "disconnect_timeout", &[])
                .await?;
            self.send_packet(conf_out::DisconnectPacket { reason })
                .await?;
            return Err(Error::MissedKeepAlive);
        }
        debug!("sending next keep-alive packet");
        let id = crypto::generate_keep_alive();
        self.keep_alive_id = Some(id);
        let packet = conf_out::KeepAlivePacket { id };
        self.send_packet(packet).await?;
        Ok(())
    }

    #[instrument(skip_all)]
    fn handle_keep_alive(&mut self, id: u64) {
        if self.keep_alive_id == Some(id) {
            self.keep_alive_id = None;
        } else {
            debug!(id = id, "keep alive packet id unknown");
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
        // The Minecraft (Java) protocol starts with the client sending a handshake packet to the server.
        // The handshake packet, most notably, contains the `next_state` field which indicates whether
        // the client intends to ask for the server `status`, want to `login` or `transfer`.
        // The handshake packet also contains some basic client information, such as the Minecraft
        // version (by the `protocol_version`), and the server address the client used to connect to
        // the server.
        debug!("awaiting handshake packet");
        let packet = self.read_next_packet().await?;
        let handshake = match_packet! { packet;
            packet = hand_in::HandshakePacket => packet,
        };

        // When the client asks for the server status, then it sends the status request packet next.
        // We then use the status adapter to get the server status based on the client and server
        // information included in the previous handshake packet.
        // Lastly, the client and server exchange a ping-pong exchange for the client to determine
        // the server latency. The latency is displayed as the server ping in the client server list.
        // The connection is automatically closed after the exchange.
        if handshake.next_state == State::Status {
            debug!("awaiting status request packet");
            let packet = self.read_next_packet().await?;
            let _ = match_packet! { packet;
                packet = status_in::StatusRequestPacket => packet,
            };

            debug!("getting status from supplier");
            let status = self
                .adapters
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
            let packet = self.read_next_packet().await?;
            let ping = match_packet! { packet;
                packet = status_in::PingPacket => packet,
            };

            debug!("sending pong packet");
            self.send_packet(status_out::PongPacket {
                payload: ping.payload,
            })
            .await?;

            return Ok(());
        }

        // In case the client wants to `login` or `transfer`, then we transition from the handshake
        // phase into the login phase. The login phase begins with the client sending the login start
        // packet. It contains (unverified) information about the Minecraft user (using the client).
        // We also ask for any Passage session cookie that may be set on the current client session.
        // The session cookie contains (unverified) session information such as any previous OpenTelemetry
        // trace id. This cookie is, by design, neither signed nor obfuscated.

        // handle login request
        debug!("awaiting login start packet");
        let packet = self.read_next_packet().await?;
        let mut login_start = match_packet! { packet;
            packet = login_in::LoginStartPacket => packet,
        };

        // check session
        debug!("sending session cookie request packet");
        self.send_packet(login_out::CookieRequestPacket {
            key: SESSION_COOKIE_KEY.to_string(),
        })
        .await?;

        debug!("awaiting session cookie response packet");
        let packet = self.read_next_packet().await?;
        let session_packet = match_packet! { packet;
            packet = login_in::CookieResponsePacket => packet,
        };
        let session_cookie = session_packet.decode::<SessionCookie>()?;

        // In case the client asked to be transferred, then we also request the Passage auth cookie.
        // The auth cookie contains (verified) information about the client session signed using some
        // shared secret. If the same shared secret is also configured for Passage and the signature
        // is valid and the cookie has not expired, then Passage uses the included user information
        // and skips the Mojang authentication.
        // By design, Passage does not punish clients for presenting mismatching user information
        // between the login start packet and auth cookie. Passage instead uses auth cookie information.
        // Generally, Passage tries to prevent states that result in clients getting disconnected.

        // handle transfer request (verifies auth cookie)
        let mut should_authenticate = true;
        let mut profile_properties = vec![];
        'transfer: {
            if handshake.next_state == State::Transfer {
                if self.config.auth_secret.is_none() {
                    debug!("no auth secret configured, skipping auth cookie");
                    break 'transfer;
                }

                debug!("sending auth cookie request packet");
                self.send_packet(login_out::CookieRequestPacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                })
                .await?;

                debug!("awaiting auth cookie response packet");
                let packet = self.read_next_packet().await?;
                let cookie = match_packet! { packet;
                    packet = login_in::CookieResponsePacket => packet,
                };

                let Some(signed) = cookie.payload else {
                    debug!("no auth cookie received, skipping auth cookie");
                    break 'transfer;
                };

                let Some(secret) = &self.config.auth_secret else {
                    debug!("no auth secret configured, skipping auth cookie");
                    break 'transfer;
                };

                let (ok, message) = verify(&signed, secret.as_bytes());
                if !ok {
                    debug!("invalid auth cookie signature received, skipping auth cookie");
                    break 'transfer;
                }

                let cookie = serde_json::from_slice::<AuthCookie>(message)?;
                let expires_at = cookie.timestamp + self.config.auth_cookie_expiry;
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

        // Next, Passages creates a shared secret between the server and client that will be used
        // to encrypt the connection.
        // First, we generate a new cryptographically secure `verify_token`. This token is then exchanged
        // with the client through the unencrypted connection together with the server public key. The
        // public key is generated on startup for each Passage instance (i.e., each Passage instance
        // poses as a separate Minecraft server).
        // In case the previous transfer step did not succeed, then we tell the client to authenticate
        // their login request against the Mojang API.

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
        let packet = self.read_next_packet().await?;
        let encrypt = match_packet! { packet;
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

        // If necessary, we now also make an authentication request using the authentication adapter.
        // By default, this entails making an HTTP request against the Mojang API.
        // as before, Passage does not punish clients for presenting mismatching user information
        // between the login start packet and adapter. Passage instead uses adapter information.
        // Generally, Passage tries to prevent states that result in clients getting disconnected.

        // handle authentication if not already authenticated by the token
        if should_authenticate {
            debug!("authenticating user");
            let request_start = Instant::now();
            let auth_response = self
                .adapters
                .authenticate(
                    &self.client_address,
                    (&handshake.server_address, handshake.server_port),
                    handshake.protocol_version as Protocol,
                    (&login_start.user_name, &login_start.user_id),
                    &shared_secret,
                    &crypto::ENCODED_PUB,
                )
                .await
                .inspect_err(|err| {
                    // track request failed
                    error!(err = %err, "authentication request failed");
                    metrics::authentication_request_duration::record(request_start, "failed");
                })?;
            metrics::authentication_request_duration::record(request_start, "success");

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

        // Before completing the login phase, the target selection is initiated using the now verified
        // client and user information. At the end it will present a single target representing a Minecraft
        // gameserver the client should transfer to.
        // The selection runs in a separate thread to not block any client IO.
        // The target selection uses three adapters, a target discovery, which gives the set of all
        // targets, a traget filtering which removes all targets the client should not transfer to,
        // and lastly, a targets strategy that selects a single target.

        // create a cancellation token with a drop guard
        let target_token = CancellationToken::new();
        let _target_token_guard = target_token.clone().drop_guard();

        // start the target selection task (stops using a stop token)
        let adapters = self.adapters.clone();
        let client_address = self.client_address;
        let server_address = handshake.server_address.clone();
        let server_port = handshake.server_port;
        let protocol = handshake.protocol_version as Protocol;
        let user_name = login_start.user_name.clone();
        let user_id = login_start.user_id;
        let mut target_join = tokio::spawn(async move {
            tokio::select! {
                _ = target_token.cancelled() => Ok(None),
                maybe_target = adapters.select(&client_address, (&server_address, server_port), protocol, (&user_name, &user_id)) => maybe_target
            }
        });

        // Next, the login phase completes by receiving the login acknowledged packet. Starting with
        // the configuration phase, the protocol becomes less strict. We primarily wait for the target
        // selection to complete such that we can transfer the client to the actual Minecraft server.
        // However, as this may take a while, we have to exchange periodic keep-alive packets with the
        // client. The server has to send one keep-alive packet at least ever 20 seconds. The client
        // then has 15 seconds to send an answer using the keep-alive id.
        // At the same time we wait for the client information packet of the client. It most notably
        // contains the client locale which we use to translate the disconnect packets.

        // If the target selection completes successfully but does not provide a target, then we send
        // a translated disconnect packet to the client and close the connection.

        debug!("awaiting login acknowledged packet");
        let packet = self.read_next_packet().await?;
        let _ = match_packet! { packet;
            packet = login_in::LoginAcknowledgedPacket => packet,
        };

        // await the target from the target task
        let interval_duration = Duration::from_secs(KEEP_ALIVE_INTERVAL);
        let mut interval =
            tokio::time::interval_at(Instant::now().add(interval_duration), interval_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        debug!("selecting target");
        let target = loop {
            tokio::select! {
                biased;

                // Await any client packet. The packet is parsed in the block such that the future
                // is not canceled, and we gain access to the connection object again.
                maybe_length = self.next_packet() => {
                    let packet = self.read_packet(maybe_length?).await?;
                    match_packet! { packet;
                        // Ignore allowed packets but do nothing
                        _ = conf_in::PluginMessagePacket => continue,
                        _ = conf_in::ResourcePackResponsePacket => continue,
                        _ = conf_in::CookieResponsePacket => continue,

                        // Update the client locale whenever a client information packet is received
                        packet = conf_in::ClientInformationPacket => {
                            self.client_locale = Some(packet.locale.clone());
                            metrics::client_locale::inc(packet.locale);
                            metrics::client_view_distance::record(
                                u64::try_from(packet.view_distance).unwrap_or(0u64),
                            );
                            continue;
                        },

                        // Handle keep alive packets
                        packet = conf_in::KeepAlivePacket => {
                            self.handle_keep_alive(packet.id);
                            continue;
                        },
                    }
                },

                // Send periodic keep alive
                _ = interval.tick() => {
                    self.send_keep_alive().await?;
                    continue;
                },

                // Await target selection to complete. This is only polled after the client
                // information packet has been received at least once.
                maybe_target = &mut target_join , if self.client_locale.is_some() => {
                    let target = maybe_target
                        .map_err(|err| passage_adapters::Error::FailedFetch {
                            adapter_type: "adapters",
                            cause: Box::new(err),
                        })??;
                    break target
                },
            }
        };

        // disconnect if not target found
        let Some(target) = target else {
            debug!("no transfer target found");
            let reason = self
                .adapters
                .localize(self.client_locale.as_deref(), "disconnect_no_target", &[])
                .await?;
            debug!("sending disconnect packet");
            self.send_packet(conf_out::DisconnectPacket { reason })
                .await?;
            return Err(Error::NoTargetFound);
        };

        // If no auth cookie was set and the shared secret for the auth cookie is set, then we set
        // the auth cookie using the (verified) user information gained from the Mojang API.
        // We also set the session cookie if it is not set. It includes the current OpenTelemetry
        // trace id as well as additional client and user information.
        // Lastly, we transfer the user to the selected target.

        // write auth cookie
        'auth_cookie: {
            if should_authenticate {
                debug!("writing auth cookie");

                let Some(secret) = &self.config.auth_secret else {
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
                    payload: sign(&auth_payload, secret.as_bytes()),
                })
                .await?;
            }
        }

        // set session id if not exist (does not override the session fields)
        if session_cookie.is_none() {
            debug!("sending session cookie packet");
            let trace_id = tracing::Span::current()
                .context()
                .span()
                .span_context()
                .trace_id()
                .to_string();
            self.send_packet(conf_out::StoreCookiePacket {
                key: SESSION_COOKIE_KEY.to_string(),
                payload: serde_json::to_vec(&SessionCookie {
                    id: Uuid::new_v4(),
                    server_address: handshake.server_address.clone(),
                    server_port: handshake.server_port,
                    trace_id: Some(trace_id),
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
