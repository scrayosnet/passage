use crate::config::Config;
use crate::cookie::{
    AUTH_COOKIE_KEY, AuthCookie, CookieDecodeExt, CookieEncodeExt, SESSION_COOKIE_KEY,
    SessionCookie,
};
pub(crate) use crate::error::Error;
use crate::{crypto, metrics};
use futures::{SinkExt, StreamExt};
use opentelemetry::trace::TraceContextExt;
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::filter::FilterAdapter;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters::{
    Adapters, Protocol, Reason, ServerStatus, discovery::DiscoveryAdapter, status::StatusAdapter,
    strategy::StrategyAdapter,
};
use passage_packets::codec::{PacketCodec, PacketFrame};
use passage_packets::configuration::clientbound as conf_out;
use passage_packets::configuration::serverbound as conf_in;
use passage_packets::handshake::serverbound as hand_in;
use passage_packets::login::clientbound as login_out;
use passage_packets::login::serverbound as login_in;
use passage_packets::status::clientbound as status_out;
use passage_packets::status::serverbound as status_in;
use passage_packets::{State, VarInt, match_packet, writer::WritePacket};
use std::fmt::Debug;
use std::net::SocketAddr;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, debug, field, info, instrument, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

/// The max packet length in bytes. Larger packets are rejected.
pub const DEFAULT_MAX_PACKET_LENGTH: VarInt = 10_000;

/// The default auth cookie expiry time in seconds.
pub const DEFAULT_AUTH_COOKIE_EXPIRY: u64 = 6 * 60 * 60;

/// The interval in seconds at which keep-alive packets are sent. Has to be between 15 and 20 seconds,
/// such that at most one keep-alive packet is in transit at any point.
pub const KEEP_ALIVE_INTERVAL: u64 = 16;

/// A connection wraps a packet stream and implements the Minecraft (Java) protocol. The connection
/// is automatically closed at the next appropriate instant once the cancellation token has been canceled.
pub struct Connection<S, Stat, Disc, Filt, Stra, Auth, Loca> {
    /// The packet stream. This is used to send and receive packets.
    stream: Framed<S, PacketCodec>,

    /// The adapters bundle. This is used to get the server status, the user profile, and the target server.
    adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,

    /// The static configuration of all Passage connections.
    config: Config,

    /// The address of the client. This is passed on to the adapters.
    client_address: SocketAddr,

    /// The shutdown token. When the cancellation token is canceled, the connection is closed. Depending
    /// on the time at which the token is canceled, an appropriate disconnect message is sent to the client.
    shutdown: CancellationToken,

    /// The ID of the last keep-alive packet sent. This is used to detect if a keep-alive packet is
    /// answered before the next is sent.
    keep_alive_id: Option<u64>,

    /// The locale of the client. This is used to localize the disconnect reason.
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
    /// Creates a new [`Connection`]. The stream is wrapped in a [`Framed`] with [`PacketCodec`] which
    /// encodes and decodes packets. Any stream prefixes, such as a proxy protocol, has to be handled
    /// beforehand.
    pub fn new(
        stream: S,
        adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,
        config: Config,
        client_address: SocketAddr,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            stream: Framed::new(stream, PacketCodec::new(config.max_packet_length)),
            adapters,
            config,
            shutdown,
            client_address,
            keep_alive_id: None,
            client_locale: None,
        }
    }

    /// Gets the server status from the [`StatusAdapter`]. If the status is not found, then the
    /// [`Error::ConnectionClosed`] error is returned. If the adapter errors, the connection is closed.
    /// If the adapter gives no status, then a default status is sent.
    #[instrument(skip_all)]
    async fn get_status(
        &self,
        handshake: &hand_in::HandshakePacket,
    ) -> Result<ServerStatus, Error> {
        // Build a new status request future.
        let status_future = async {
            self.adapters
                .status(
                    &self.client_address,
                    (&handshake.server_address, handshake.server_port),
                    handshake.protocol_version,
                )
                .await
        };

        // Wait for the status adapter to complete. Stop if the connection is shutdown. At this point,
        // we cannot send any disconnect packet.
        let shutdown = self.shutdown.clone();
        let status = tokio::select! {
            status = status_future => status?,
            _ = shutdown.cancelled() => return Err(Error::ConnectionClosed),
        };

        // Handle status not found.
        Ok(status.unwrap_or_default())
    }

    /// Gets the user profile from the [`AuthenticationAdapter`]. If the adapter errors, the connection
    /// is closed. If the adapter gives no profile, then a disconnect packet is sent and the connection
    /// is closed.
    #[instrument(skip_all)]
    async fn get_profile(
        &mut self,
        handshake: &hand_in::HandshakePacket,
        login_start: &login_in::LoginStartPacket,
        shared_secret: &[u8],
    ) -> Result<Profile, Error> {
        // Build a new status request future.
        let profile_future = async {
            self.adapters
                .authenticate(
                    &self.client_address,
                    (&handshake.server_address, handshake.server_port),
                    handshake.protocol_version as Protocol,
                    (&login_start.user_name, &login_start.user_id),
                    shared_secret,
                    &crypto::ENCODED_PUB,
                )
                .await
        };

        // Wait for the status adapter to complete. Stop if the connection is shutdown.
        let shutdown = self.shutdown.clone();
        let profile = tokio::select! {
            profile = profile_future => profile?,
            _ = shutdown.cancelled() => Reason::None(Some("disconnect_timeout".to_string())),
        };

        // Handle profile not found.
        match profile {
            Reason::Some(profile) => Ok(profile),
            Reason::None(key) => {
                info!("profile not found, disconnecting");
                let reason = self
                    .adapters
                    .localize(
                        self.client_locale.as_deref(),
                        key.as_deref().unwrap_or("disconnect_unauthenticated"),
                        &[],
                    )
                    .await?;
                self.send_packet(login_out::DisconnectPacket { reason })
                    .await?;
                Err(Error::ConnectionClosed)
            }
        }
    }

    /// Awaits the next packet from the stream or for the cancellation token to be canceled. If the
    /// cancellation token is canceled, then the connection is closed.
    #[instrument(skip_all, fields(packet_length = field::Empty, packet_id = field::Empty))]
    async fn next_packet(&mut self) -> Result<PacketFrame, Error> {
        // Wait for the next packet to arrive. Stop if the connection is shutdown.
        let shutdown = self.shutdown.clone();
        let frame = tokio::select! {
            frame = self.stream.next().instrument(tracing::info_span!("read_packet", otel.kind = "server")) => frame,
            // TODO could send a disconnect packet in the login and configuration phase.
            _ = shutdown.cancelled() => return Err(Error::ConnectionClosed),
        };

        // Check if a packet was received, otherwise close the connection.
        let frame = frame.ok_or_else(|| Error::ConnectionClosed)??;
        tracing::Span::current().record("packet_length", frame.length);
        tracing::Span::current().record("packet_id", frame.id);
        Ok(frame)
    }

    #[instrument(skip_all)]
    async fn send_packet<T: WritePacket + Send + Sync + Debug>(
        &mut self,
        packet: T,
    ) -> Result<(), Error> {
        self.stream
            .send(packet)
            .instrument(tracing::info_span!("write_packet", otel.kind = "server"))
            .await?;
        Ok(())
    }

    // TODO remove?
    #[instrument(skip_all)]
    async fn send_keep_alive(&mut self) -> Result<(), Error> {
        debug!("checking that keep-alive packet was received");
        if self.keep_alive_id.is_some() {
            info!("keep-alive missed, disconnecting");
            let reason = self
                .adapters
                .localize(self.client_locale.as_deref(), "disconnect_timeout", &[])
                .await?;
            self.send_packet(conf_out::DisconnectPacket { reason })
                .await?;
            return Err(Error::ConnectionClosed);
        }
        debug!("sending next keep-alive packet");
        let id = crypto::generate_keep_alive();
        self.keep_alive_id = Some(id);
        let packet = conf_out::KeepAlivePacket { id };
        self.send_packet(packet).await?;
        Ok(())
    }

    // TODO remove?
    fn handle_keep_alive(&mut self, id: u64) {
        if self.keep_alive_id == Some(id) {
            self.keep_alive_id = None;
        } else {
            debug!(id = id, "keep alive packet id unknown");
        }
    }

    #[instrument(skip_all)]
    fn apply_encryption(&mut self, shared_secret: &[u8]) {
        debug!("enabling encryption");
        self.stream
            .codec_mut()
            .encrypt(shared_secret)
            .expect("Secret key is always generated to be valid");
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
        let packet = self.next_packet().await?;
        let handshake = match_packet! { packet,
            packet = hand_in::HandshakePacket => packet,
            (unexpected, _) => {
                info!(unexpected = ?unexpected, "expected handshake packet, closing connection");
                return Err(Error::ConnectionClosed);
            }
        }?;
        metrics::handshake_states::inc(handshake.next_state);

        // When the client asks for the server status, then it sends the status request packet next.
        // We then use the status adapter to get the server status based on the client and server
        // information included in the previous handshake packet.
        // Lastly, the client and server exchange a ping-pong exchange for the client to determine
        // the server latency. The latency is displayed as the server ping in the client server list.
        // The connection is automatically closed after the exchange.
        if handshake.next_state == State::Status {
            debug!("awaiting status request packet");
            let packet = self.next_packet().await?;
            let _ = match_packet! { packet,
                packet = status_in::StatusRequestPacket => packet,
                (unexpected, _) => {
                    info!(unexpected = ?unexpected, "expected status packet, closing connection");
                    return Err(Error::ConnectionClosed);
                }
            }?;

            debug!("getting status from supplier");
            let status = self.get_status(&handshake).await?;

            debug!("sending status response packet");
            self.send_packet(status_out::StatusResponsePacket::try_from(&status)?)
                .await?;

            debug!("awaiting ping packet");
            let packet = self.next_packet().await?;
            let ping = match_packet! { packet,
                packet = status_in::PingPacket => packet,
                (unexpected, _) => {
                    info!(unexpected = ?unexpected, "expected ping packet, closing connection");
                    return Err(Error::ConnectionClosed);
                }
            }?;

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
        let packet = self.next_packet().await?;
        let mut login_start = match_packet! { packet,
            packet = login_in::LoginStartPacket => packet,
            (unexpected, _) => {
                info!(unexpected = ?unexpected, "expected login start packet, closing connection");
                return Err(Error::ConnectionClosed);
            }
        }?;

        // check session
        debug!("sending session cookie request packet");
        self.send_packet(login_out::CookieRequestPacket {
            key: SESSION_COOKIE_KEY.to_string(),
        })
        .await?;

        debug!("awaiting session cookie response packet");
        let packet = self.next_packet().await?;
        let session_packet = match_packet! { packet,
            packet = login_in::CookieResponsePacket => packet,
            (unexpected, _) => {
                info!(unexpected = ?unexpected, "expected session packet, closing connection");
                return Err(Error::ConnectionClosed);
            }
        }?;

        debug!("decoding the session cookie");
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
                let packet = self.next_packet().await?;
                let cookie = match_packet! { packet,
                    packet = login_in::CookieResponsePacket => packet,
                    (unexpected, _) => {
                        info!(unexpected = ?unexpected, "expected auth packet, closing connection");
                        return Err(Error::ConnectionClosed);
                    }
                }?;

                let Some(secret) = &self.config.auth_secret else {
                    debug!("no auth secret configured, skipping auth cookie");
                    break 'transfer;
                };

                let Some(cookie) = cookie.decode_verified::<AuthCookie>(secret.as_bytes())? else {
                    debug!("decoding or verifying failed, skipping auth cookie");
                    break 'transfer;
                };

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
        let packet = self.next_packet().await?;
        let encrypt = match_packet! { packet,
            packet = login_in::EncryptionResponsePacket => packet,
            (unexpected, _) => {
                info!(unexpected = ?unexpected, "expected encryption packet, closing connection");
                return Err(Error::ConnectionClosed);
            }
        }?;

        // decrypt the shared secret and verify the token
        let shared_secret = crypto::decrypt(&crypto::KEY_PAIR.0, &encrypt.shared_secret)?;
        let decrypted_verify_token = crypto::decrypt(&crypto::KEY_PAIR.0, &encrypt.verify_token)?;

        // verify the token is correct
        debug!("verifying verify token");
        if !crypto::verify_token(verify_token, &decrypted_verify_token) {
            info!("received invalid verify token, closing connection");
            return Err(Error::ConnectionClosed);
        }

        // If necessary, we now also make an authentication request using the authentication adapter.
        // By default, this entails making an HTTP request against the Mojang API.
        // as before, Passage does not punish clients for presenting mismatching user information
        // between the login start packet and adapter. Passage instead uses adapter information.
        // Generally, Passage tries to prevent states that result in clients getting disconnected.

        // enable encryption for the connection using the shared secret
        self.apply_encryption(&shared_secret);

        // handle authentication if not already authenticated by the token
        if should_authenticate {
            debug!("authenticating user");
            let profile = self
                .get_profile(&handshake, &login_start, &shared_secret)
                .await?;

            // update state for actual use info
            login_start.user_name = profile.name;
            login_start.user_id = profile.id;
            profile_properties = profile.properties;
        }

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

        // start the target selection task
        let adapters = self.adapters.clone();
        let client_address = self.client_address;
        let server_address = handshake.server_address.clone();
        let server_port = handshake.server_port;
        let protocol = handshake.protocol_version as Protocol;
        let user_name = login_start.user_name.clone();
        let user_id = login_start.user_id;
        let shutdown = self.shutdown.clone();
        let mut target_join = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.cancelled() => Ok(Reason::None(Some("disconnect_timeout".to_string()))),
                maybe_target = adapters.select(&client_address, (&server_address, server_port), protocol, (&user_name, &user_id)) => {
                    maybe_target
                }
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
        let packet = self.next_packet().await?;
        let _ = match_packet! { packet,
            packet = login_in::LoginAcknowledgedPacket => packet,
            (unexpected, _) => {
                info!(unexpected = ?unexpected, "expected login ack. packet, closing connection");
                return Err(Error::ConnectionClosed);
            }
        }?;

        // await the target from the target task
        let interval_duration = Duration::from_secs(KEEP_ALIVE_INTERVAL);
        let mut interval =
            tokio::time::interval_at(Instant::now().add(interval_duration), interval_duration);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        debug!("selecting target");
        let target = loop {
            tokio::select! {
                biased;

                // Await any client packet. This method should be cancellation safe.
                maybe_packet = self.next_packet() => {
                    match_packet! { maybe_packet?,
                        // Ignore allowed packets but do nothing
                        _ = conf_in::PluginMessagePacket => continue,
                        _ = conf_in::ResourcePackResponsePacket => continue,
                        _ = conf_in::CookieResponsePacket => continue,

                        // Update the client locale whenever a client information packet is received
                        packet = conf_in::ClientInformationPacket => {
                            let packet = packet?;
                            metrics::client_locales::inc(packet.locale.clone());
                            metrics::client_view_distances::record(packet.view_distance as u64);
                            self.client_locale = Some(packet.locale);
                            continue;
                        },

                        // Handle keep alive packets
                        packet = conf_in::KeepAlivePacket => {
                            self.handle_keep_alive(packet?.id);
                            continue;
                        },

                        // Throw on any other packet
                        (unexpected, _) => {
                            info!(unexpected = ?unexpected, "expected client packets, closing connection");
                            return Err(Error::ConnectionClosed);
                        }
                    }
                },

                // Send periodic keep alive
                _ = interval.tick() => {
                    self.send_keep_alive().await?;
                    continue;
                },

                // Await target selection to complete. This is only polled after the client
                // information packet has been received at least once.
                target = &mut target_join , if self.client_locale.is_some() => {
                    let target = target
                        .map_err(|err| passage_adapters::Error::FailedFetch {
                            adapter_type: "adapters",
                            cause: Box::new(err),
                        })??;
                    break target
                },
            }
        };

        // disconnect if not target found
        let target = match target {
            Reason::Some(target) => target,
            Reason::None(reason) => {
                info!("no transfer target found, disconnecting");
                let reason = self
                    .adapters
                    .localize(
                        self.client_locale.as_deref(),
                        reason.as_deref().unwrap_or("disconnect_no_target"),
                        &[],
                    )
                    .await?;
                self.send_packet(conf_out::DisconnectPacket { reason })
                    .await?;
                return Err(Error::ConnectionClosed);
            }
        };

        // If the shared secret for the auth cookie is set, then we set a new auth cookie using the
        // (verified) user information gained from the Mojang API.
        // We also set the session cookie if it is not set. It includes the current OpenTelemetry
        // trace id as well as additional client and user information.
        // Lastly, we transfer the user to the selected target.

        // write auth cookie
        if let Some(secret) = &self.config.auth_secret {
            debug!("writing auth cookie");

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

            debug!("sending auth cookie packet");
            self.send_packet(conf_out::StoreCookiePacket::encode_signed(
                secret.as_bytes(),
                &cookie,
            )?)
            .await?;
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

            let cookie = SessionCookie {
                id: Uuid::new_v4(),
                server_address: handshake.server_address.clone(),
                server_port: handshake.server_port,
                trace_id: Some(trace_id),
            };

            self.send_packet(conf_out::StoreCookiePacket::encode(&cookie)?)
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
