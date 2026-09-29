use crate::cookie::{AuthCookie, Cookie, SessionCookie};
use crate::router::state::{State, Step};
use crate::router::utils::ConnRefExt;
use crate::{crypto, metrics};
use anyhow::{Context, anyhow};
use opentelemetry::global;
use opentelemetry::trace::TraceContextExt;
use passage_adapters::authentication::AuthenticationAdapter;
use passage_core::codec::{Aes128Cfb8, SECRET_LEN};
use passage_core::connection::{ConnRef, DispatchError};
use passage_core::packet::{configuration, handshake, login, status};
use passage_core::wire::{ByteString, Bytes};
use passage_core::{Phase, versions};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::{Span, debug, field, info, instrument, trace, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use uuid::Uuid;

/// Expects the provided step, unwrapping it using the expression. The step is replaced with `working`
/// and should be replaced with a different step before the client sends the next packet. Returns
/// and `unexpected_step` is the step does not match.
macro_rules! expect_step {
    // Match a pattern and return an expression built from its bindings
    ($conn:expr, $pat:pat => $out:expr) => {
        match $conn.state.set_working(stringify!($pat)) {
            $pat => Ok($out),
            other => Err(DispatchError::peer(
                "unexpected_step",
                anyhow::anyhow!("Expected step `{}`, got `{:?}`", stringify!($pat), other),
            )),
        }
    };
}

/// The interval between keep alive packets. The client drops the connection after 20 seconds
/// without one.
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(16);

/// How long before the connection's deadline the client is told that it ran out of time.
const GRACE: Duration = Duration::from_secs(1);

/// Everything this connection does on a clock: keep the client alive while it waits for a target,
/// and tell it why if the connection is about to go away. The connection has no timer of its own
/// beyond its deadline, so this runs for as long as the connection does.
#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        reason = field::Empty,
    ),
)]
pub(crate) async fn on_open(conn: ConnRef<'_, State>) -> crate::Result<(), DispatchError> {
    let shutdown = conn.with(|c| c.shutdown().clone());
    let deadline = conn.with(|c| c.deadline());

    let reason = tokio::select! {
        result = keep_alives(conn) => return result,
        () = shutdown.cancelled() => "disconnect_restart",
        () = expiring(deadline) => "disconnect_timeout",
    };
    Span::current().record("reason", reason);
    farewell(conn, reason).await
}

/// Exchanges keep alives while the client waits for its target.
async fn keep_alives(conn: ConnRef<'_, State>) -> crate::Result<(), DispatchError> {
    loop {
        tokio::time::sleep(KEEP_ALIVE_INTERVAL).await;

        // Keep alives belong to the step that waits for a target, and to no other.
        let Some(pending) = conn.with(|c| match &c.state.step {
            Step::Transfer { keep_alive, .. } => Some(*keep_alive),
            _ => None,
        }) else {
            continue;
        };

        // A keep alive that was never answered means the client is gone.
        if pending.is_some() {
            debug!(sent = ?pending, "the client never answered its keep alive");
            return farewell(conn, "disconnect_timeout").await;
        }

        // Send the next keep alive packet, and remember what the client has to answer with.
        let id = crypto::generate_keep_alive();
        trace!(id, "sending keep alive");
        conn.with(|c| {
            c.send(configuration::ServerKeepAlivePacket::new(id))?;
            let informed = expect_step!(c, Step::Transfer { informed, .. } => informed)?;
            c.state.step = Step::Transfer {
                keep_alive: Some(id),
                informed,
            };
            Ok::<_, DispatchError>(())
        })?;
    }
}

/// Resolves [`GRACE`] before `deadline`, or never if there is none.
async fn expiring(deadline: Option<Instant>) {
    match deadline {
        Some(at) => tokio::time::sleep_until(at - GRACE).await,
        None => std::future::pending().await,
    }
}

/// Tells the client why it is being disconnected, then ends the connection with `reason`.
///
/// The phase decides which disconnect packet to use; a connection that never got past the handshake
/// has none to use, so it is simply dropped.
#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        reason = reason,
    ),
)]
async fn farewell(
    conn: ConnRef<'_, State>,
    reason: &'static str,
) -> crate::Result<(), DispatchError> {
    let phase = conn.phase();
    if !matches!(phase, Phase::Login | Phase::Configuration) {
        trace!(
            ?phase,
            "no disconnect packet exists in this phase, dropping"
        );
        return Err(DispatchError::peer(reason, anyhow!("{reason}")));
    }

    let message = conn.localize(reason, &[]).await;
    metrics::disconnects::inc(reason);
    debug!(reason, ?phase, "disconnecting client");
    conn.with(|c| {
        // A cancelled shutdown token would otherwise refuse the write it caused.
        c.detach();
        match phase {
            Phase::Login => c.send(login::ServerDisconnectPacket::text(message))?,
            _ => c.send(configuration::ServerDisconnectPacket::text(message))?,
        }
        c.fail(DispatchError::peer(reason, anyhow!("{reason}")));
        Ok(())
    })
}

/// The packet that decides what the rest of the connection is: which version it speaks, which
/// hostname it dialed, and therefore which route's adapters answer it.
#[instrument(
    level = "info",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        client.address = field::Empty,
        client.port = field::Empty,
        server.address = %packet.server_address,
        server.port = packet.server_port,
        network.protocol.version = %packet.protocol_version,
        packet.intent = ?packet.next_state,
        route = field::Empty,
    ),
)]
pub(crate) async fn on_handshake_intention_packet(
    conn: ConnRef<'_, State>,
    packet: handshake::ClientIntentionPacket,
) -> crate::Result<(), DispatchError> {
    metrics::handshake_states::inc(packet.next_state);
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::Intention => ())?;

        // Update the connection state.
        c.set_version(packet.protocol_version);
        c.set_phase(packet.next_state.into());

        // Update the custom state.
        c.state.find_route(&packet.server_address);
        c.state.client.protocol_version = packet.protocol_version;
        c.state.client.server_address = packet.server_address;
        c.state.client.server_port = packet.server_port;

        let span = Span::current();
        span.record(
            "client.address",
            field::display(c.state.client.address.ip()),
        );
        span.record("client.port", c.state.client.address.port());

        // A hostname nobody configured has no adapters to answer it with, so there is nothing this
        // connection could do from here on.
        let Some(route) = c.state.route() else {
            return Err(DispatchError::peer(
                "no_route",
                anyhow!("no route matches `{}`", c.state.client.server_address),
            ));
        };
        span.record("route", field::display(&route.hostname));
        c.state.step = packet.next_state.into();
        Ok(())
    })
}

/// Hands a cookie the client sent back to the handler that asked for it, and returns the connection
/// to the step that asked. A cookie nobody is waiting for, or one for another key, is the client
/// speaking out of turn.
#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        cookie.key = key,
        cookie.length = ?payload.as_ref().map(|p| p.len()),
    )
)]
fn accept_cookie(
    conn: ConnRef<'_, State>,
    key: &str,
    payload: Option<Bytes>,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        let (expected, sender, resume) = expect_step!(c, Step::Cookie { key: expected, sender, resume } => (expected, sender, resume))?;
        if expected != key {
            return Err(DispatchError::peer(
                "unexpected_cookie",
                anyhow!("no cookie `{key}` was asked for at step `{resume:?}`"),
            ));
        };

        c.state.step = *resume;
        let _ = sender.send(payload);
        Ok(())
    })
}

#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.key = %packet.key,
    )
)]
pub(crate) async fn on_login_cookie_response(
    conn: ConnRef<'_, State>,
    packet: login::ClientCookieResponsePacket,
) -> crate::Result<(), DispatchError> {
    accept_cookie(conn, &packet.key, packet.payload)
}

#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.key = %packet.key,
    )
)]
pub(crate) async fn on_configuration_cookie_response(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientCookieResponsePacket,
) -> crate::Result<(), DispatchError> {
    accept_cookie(conn, &packet.key, packet.payload)
}

/// The brand and any other channel the client opens with. Passage answers none of them, but a
/// client that sends one has not done anything wrong.
#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.channel = %packet.channel,
        packet.payload.length = %packet.data.len(),
    )
)]
pub(crate) async fn on_configuration_custom_payload(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientCustomPayloadPacket,
) -> crate::Result<(), DispatchError> {
    let _ = (conn, packet);
    trace!("ignoring plugin message");
    Ok(())
}

/// Passage pushes no resource packs, so an answer about one is ignored rather than refused.
#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.uuid = %packet.uuid,
        packet.result = ?packet.result,
    )
)]
pub(crate) async fn on_configuration_resource_pack(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientResourcePackPacket,
) -> crate::Result<(), DispatchError> {
    let _ = (conn, packet);
    trace!("ignoring resource pack response");
    Ok(())
}

#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
    )
)]
pub(crate) async fn on_status_request_packet(
    conn: ConnRef<'_, State>,
    packet: status::ClientStatusRequestPacket,
) -> crate::Result<(), DispatchError> {
    _ = packet;
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::StatusRequest => ())?;
        Ok::<(), DispatchError>(())
    })?;

    // Query the status adapter based on the selected route. Then, send the status response to the
    // client. Any adapter errors are handled directly.
    let status = match conn.status().await {
        Ok(status) => status.unwrap_or_default(),
        Err(err) => {
            if err.is_rejected() {
                debug!(err = %err, "the status adapter rejected the request");
            } else {
                warn!(err = %err, "status adapter error");
            }
            conn.close();
            return Ok(());
        }
    };

    // Send the status response to the client. Only then is a ping expected, so that a pong cannot
    // overtake the answer it is timing.
    let packet = status::ServerStatusResponsePacket::try_from(&status)
        .map_err(|err| DispatchError::internal("status_encode_error", err))?;
    conn.with(|c| {
        c.send(packet)?;
        c.state.step = Step::StatusPingRequest;
        Ok(())
    })
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.payload = %packet.payload,
    )
)]
pub(crate) async fn on_status_ping_request_packet(
    conn: ConnRef<'_, State>,
    packet: status::ClientPingRequestPacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::StatusPingRequest => ())?;

        // Send the pong response and close the connection.
        c.send(status::ServerPongResponsePacket {
            payload: packet.payload,
        })?;
        c.close();
        c.state.step = Step::Completed;
        Ok(())
    })
}

#[instrument(
    level = "info",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.player.name = %packet.name,
        packet.player.uuid = %packet.uuid,
        authenticated = field::Empty,
    )
)]
pub(crate) async fn on_login_login_start(
    conn: ConnRef<'_, State>,
    packet: login::ClientLoginStartPacket,
) -> crate::Result<(), DispatchError> {
    let (transfer, client_address, cookie_expiry) = conn.with(|c| {
        // Ensure that the connection is in the right state. The client waits from here until the
        // encryption request, apart from the cookie it may be asked for.
        let transfer = expect_step!(c, Step::LoginStart { transfer } => transfer)?;

        // Update the custom state.
        c.state.player.name = packet.name.to_string();
        c.state.player.id = packet.uuid;

        let client_address = c.state.client.address;
        let auth_cookie_expiry = c.state.auth_cookie_expiry;
        Ok::<_, DispatchError>((transfer, client_address, auth_cookie_expiry))
    })?;

    // The transfer packet arrived with the configuration phase in 1.20.5, so anything older is
    // told which version to use instead.
    if !conn.version().placed().at_least(versions::V1_20_5) {
        let preferred = conn
            .status()
            .await
            .unwrap_or_default()
            .unwrap_or_default()
            .version
            .name;
        debug!(
            preferred,
            "the client predates the transfer packet, telling it which version to use",
        );
        let reason = conn
            .localize("disconnect_unsupported", &[("preferred", preferred)])
            .await;
        return conn.with(|c| {
            c.state.step = Step::Completed;
            c.send(login::ServerDisconnectPacket { reason })?;
            c.close();
            Ok(())
        });
    };

    // Handle transfer by checking the auth cookie.
    let mut authenticated = false;
    'transfer: {
        let has_secret = conn.with(|c| c.state.secret.is_some());
        if has_secret && transfer {
            let auth_cookie = conn
                .cookie::<AuthCookie>()
                .await
                .context("Failed to get the auth cookie")?;
            let Some(auth_cookie) = auth_cookie else {
                break 'transfer;
            };

            // Check that the auth cookie is valid.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time error")
                .as_secs();
            if auth_cookie.client_addr.ip() != client_address.ip()
                || (auth_cookie.timestamp + cookie_expiry) < now
            {
                debug!(
                    age = now.saturating_sub(auth_cookie.timestamp),
                    same_address = auth_cookie.client_addr.ip() == client_address.ip(),
                    "invalid auth cookie payload received, skipping auth cookie",
                );
                break 'transfer;
            }

            // Update the auth state
            authenticated = true;
            debug!(
                player = %auth_cookie.user_name,
                id = %auth_cookie.user_id,
                "the auth cookie vouches for this player, skipping authentication",
            );
            conn.with(|c| {
                c.state.player.name = auth_cookie.user_name.to_string();
                c.state.player.id = auth_cookie.user_id;
                c.state.player.profile_properties = auth_cookie.profile_properties;
            })
        }
    }
    Span::current().record("authenticated", authenticated);

    // Whether the client is told to prove itself to Mojang first.
    let requires_session = conn
        .route()
        .is_none_or(|route| route.authentication.requires_session());

    // Send the encryption message.
    let verify_token = crypto::generate_token()
        .map_err(|err| DispatchError::internal("verify_token_error", err))?;
    conn.with(|c| {
        c.send(login::ServerEncryptionRequestPacket {
            server_id: ByteString::new(),
            public_key: crypto::ENCODED_PUB.clone(),
            verify_token: verify_token.clone(),
            should_authenticate: !authenticated && requires_session,
        })?;
        c.state.step = Step::Encrypt {
            verify_token,
            authenticated,
        };
        Ok::<_, DispatchError>(())
    })?;
    Ok(())
}

#[instrument(
    level = "info",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.shared_secret.length = packet.shared_secret.len(),
        packet.verify_token.length = packet.verify_token.len(),
        player.name = field::Empty,
        player.uuid = field::Empty,
        session.id = field::Empty,
    )
)]
pub(crate) async fn on_login_encryption_response(
    conn: ConnRef<'_, State>,
    packet: login::ClientEncryptionResponsePacket,
) -> crate::Result<(), DispatchError> {
    // Ensure that the connection is in the right state. The step contains the generated verify-token
    // and the authentication status.
    let (verify_token, authenticated) = conn.with(|c| {
        let expected = expect_step!(c, Step::Encrypt { verify_token, authenticated } => (verify_token, authenticated))?;
        Ok::<_, DispatchError>(expected)
    })?;

    // Decrypt the shared secret and the verify-token.
    let shared_secret = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.shared_secret)
        .map_err(|err| DispatchError::internal("shared_secret_decrypt_error", err))?;
    let decrypted_verify_token = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.verify_token)
        .map_err(|err| DispatchError::internal("verify_token_decrypt_error", err))?;

    // Encrypt the connection either way: the client starts encrypting the moment it sent the
    // response, so even a disconnect has to be encrypted to be readable.
    let cipher = Aes128Cfb8::new(&shared_secret).ok_or_else(|| {
        DispatchError::peer(
            "invalid_shared_secret",
            anyhow!(
                "shared secret is {} bytes, expected {SECRET_LEN}",
                shared_secret.len()
            ),
        )
    })?;
    conn.with(|c| c.encrypt(Box::new(cipher)));

    // Verify the shared secret against the keypair. If the secret is invalid, then we try to send
    // a disconnect packet. It is possible that the client cannot decrypt it, but that's still better
    // than just closing the connection.
    trace!("verifying verify token");
    if !crypto::verify_token(&verify_token, &decrypted_verify_token) {
        let message = conn.localize("disconnect_invalid_session", &[]).await;
        debug!("received invalid verify token, closing connection");
        return conn.with(|c| {
            c.state.step = Step::Completed;
            c.send(login::ServerDisconnectPacket::text(message))?;
            c.close();
            Ok(())
        });
    }

    // If the client is not authenticated, then we need to fetch the profile. We skip it if the auth
    // cookie is present (and valid).
    if !authenticated {
        // Get the profile from the route adapters. If the authentication request is rejected, then
        // the connection is closed with a disconnect packet. At this point, we only have to stop the
        // handler execution.
        let profile = match conn.authorize(&shared_secret).await {
            Ok(profile) => profile,
            Err(err) => {
                let reason = err.reason().unwrap_or("disconnect_unauthenticated");
                if err.is_rejected() {
                    debug!(reason, err = %err, "the player was not authenticated");
                } else {
                    warn!(err = %err, "profile adapter error");
                }
                let message = conn.localize(reason, &[]).await;
                return conn.with(|c| {
                    c.state.step = Step::Completed;
                    c.send(login::ServerDisconnectPacket::text(message))?;
                    c.close();
                    Ok(())
                });
            }
        };
        debug!(player = %profile.name, id = %profile.id, "authenticated the player");
        conn.with(|c| {
            c.state.player.name = profile.name.to_string();
            c.state.player.id = profile.id;
            c.state.player.profile_properties = profile.properties;
        });
    }

    let span = Span::current();
    conn.with(|c| {
        span.record("player.name", field::display(&c.state.player.name));
        span.record("player.uuid", field::display(&c.state.player.id));
    });

    // Get the session information of the client. A client without one is given a new session once
    // it is transferred.
    let session = conn.cookie::<SessionCookie>().await?;
    if let Some(session) = &session {
        span.record("session.id", field::display(&session.id));
        // A session cookie carries the trace of the connection that issued it, which is what ties
        // a transfer back to the login it came from. It is a link and not a parent: this
        // connection's trace was already rooted before the cookie could be asked for. The link sits
        // on the connection rather than on this handler, because that is the span the other end
        // issued -- the two connections meet at the same height in both traces.
        global::get_text_map_propagator(|propagator| {
            let context = propagator.extract(&session.extra);
            let linked = context.span().span_context().clone();
            if linked.is_valid() {
                conn.span().add_link(linked);
            }
        });
    }

    // Complete the login phase and prepare the target selection.
    conn.with(|c| {
        c.send(login::ServerLoginSuccessPacket {
            uuid: c.state.player.id,
            name: c.state.player.name.as_str().into(),
            properties: c.state.player.profile_properties.clone(),
            strict_error_handling: Some(true),
            session_id: session.as_ref().map(|cookie| cookie.id),
        })?;
        c.state.session = session;
        c.state.step = Step::LoginAck;
        Ok::<_, DispatchError>(())
    })
}

#[instrument(
    level = "info",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        target.identifier = field::Empty,
        target.address = field::Empty,
    )
)]
pub(crate) async fn on_login_login_acknowledged(
    conn: ConnRef<'_, State>,
    packet: login::ClientLoginAcknowledgedPacket,
) -> crate::Result<(), DispatchError> {
    // Ensure that the connection is in the right state.
    _ = packet;
    let informed = conn.with(|c| {
        expect_step!(c, Step::LoginAck => ())?;
        c.set_phase(Phase::Configuration);
        let informed = Arc::new(Notify::new());
        c.state.step = Step::Transfer {
            keep_alive: None,
            informed: Arc::clone(&informed),
        };
        Ok::<_, DispatchError>(informed)
    })?;

    // Select a target while the client sends its information. The client information packet is what
    // carries the locale, so waiting for it is what lets a rejection be localized.
    let (target, ()) = tokio::join!(conn.target(), informed.notified());
    let target = match target {
        Ok(target) => target,
        Err(err) => {
            let reason = err.reason().unwrap_or("disconnect_no_target");
            if err.is_rejected() {
                debug!(reason, err = %err, "no target was selected for the player");
            } else {
                warn!(err = %err, "target selection error");
            }
            let message = conn.localize(reason, &[]).await;
            return conn.with(|c| {
                c.state.step = Step::Completed;
                c.send(configuration::ServerDisconnectPacket::text(message))?;
                c.close();
                Ok(())
            });
        }
    };

    let span = Span::current();
    span.record("target.identifier", field::display(&target.identifier));
    span.record("target.address", field::display(target.address));

    // Store a fresh auth cookie, so the next transfer can skip authentication.
    conn.with(|c| {
        let Some(secret) = &c.state.secret else {
            return Ok(());
        };
        let cookie = AuthCookie {
            client_addr: c.state.client.address,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time error")
                .as_secs(),
            user_name: c.state.player.name.as_str().into(),
            user_id: c.state.player.id,
            target: Some(target.identifier.as_str().into()),
            profile_properties: c.state.player.profile_properties.clone(),
            extra: Default::default(),
        };
        let encoded = match cookie.encode(Some(secret)) {
            Ok(encoded) => encoded,
            Err(err) => {
                warn!(err = %err, "failed to encode auth cookie, skipping");
                return Ok(());
            }
        };
        c.send(configuration::ServerStoreCookiePacket {
            key: AuthCookie::KEY.into(),
            payload: encoded,
        })
    })?;

    // Give the client a session if it had none, so the next connection it makes is recognisable as
    // the same one. It carries the trace of the connection -- not of this handler -- which is what
    // links the two together: what the backend does after the transfer belongs beside the handlers
    // in the connection, not underneath the one packet that happened to hand it over.
    conn.with(|c| {
        if c.state.session.is_some() {
            return Ok(());
        }
        let mut extra = HashMap::new();
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&c.span().context(), &mut extra);
        });
        let cookie = SessionCookie {
            id: Uuid::new_v4(),
            server_address: c.state.client.server_address.to_string(),
            server_port: c.state.client.server_port,
            extra,
        };
        debug!(session = %cookie.id, "issuing a session to a client that had none");
        let encoded = match cookie.encode(None) {
            Ok(encoded) => encoded,
            Err(err) => {
                warn!(err = %err, "failed to encode session cookie, skipping");
                return Ok(());
            }
        };
        c.send(configuration::ServerStoreCookiePacket {
            key: SessionCookie::KEY.into(),
            payload: encoded,
        })
    })?;

    conn.with(|c| {
        c.state.step = Step::Completed;
        c.send(configuration::ServerTransferPacket {
            host: target.address.ip().to_string().as_str().into(),
            port: target.address.port(),
        })?;
        c.close();
        info!(
            player = %c.state.player.name,
            id = %c.state.player.id,
            target = %target.identifier,
            address = %target.address,
            "transferred player",
        );
        metrics::transfers::inc();
        Ok(())
    })
}

#[instrument(
    level = "info"
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.locale = %packet.locale,
        packet.view_distance = %packet.view_distance,
        packet.chat_mode = ?packet.chat_mode,
        packet.chat_colors = %packet.chat_colors,
        packet.displayed_skin_parts = ?packet.displayed_skin_parts,
        packet.main_hand = ?packet.main_hand,
        packet.enable_text_filtering = %packet.enable_text_filtering,
        packet.allow_server_listings = %packet.allow_server_listings,
        packet.particle_status = ?packet.particle_status,
    )
)]
pub(crate) async fn on_configuration_client_information(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientClientInformationPacket,
) -> crate::Result<(), DispatchError> {
    metrics::client_locales::inc(packet.locale.to_string());
    metrics::client_view_distances::record(packet.view_distance.max(0) as u64);
    conn.with(|c| {
        let (informed, keep_alive) =
            expect_step!(c, Step::Transfer { informed, keep_alive } => (informed, keep_alive))?;
        c.state.locale = Some(packet.locale.to_string());

        // The target selection waits for this, because a rejection is localized with it.
        c.state.step = Step::Transfer {
            informed: Arc::clone(&informed),
            keep_alive,
        };
        informed.notify_one();
        Ok(())
    })
}

#[instrument(
    level = "trace",
    skip_all,
    fields(
        conn.protocol = ?conn.version(),
        conn.phase = ?conn.phase(),
        packet.id = packet.id,
    )
)]
pub(crate) async fn on_configuration_keep_alive(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientKeepAlivePacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        let (keep_alive, informed) =
            expect_step!(c, Step::Transfer { keep_alive, informed } => (keep_alive, informed))?;
        if keep_alive == Some(packet.id) {
            c.state.step = Step::Transfer {
                keep_alive: None,
                informed,
            };
        } else {
            c.state.step = Step::Transfer {
                keep_alive,
                informed,
            };
            debug!(
                sent = ?keep_alive,
                received = ?packet.id,
                "received keep alive packet with invalid id",
            );
        }
        Ok(())
    })
}
