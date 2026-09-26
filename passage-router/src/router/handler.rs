use crate::cookie::{AuthCookie, Cookie, SessionCookie};
use crate::crypto;
use crate::router::state::{State, Step};
use crate::router::utils::ConnRefExt;
use anyhow::{Context, anyhow};
use passage_core::codec::NoCipher;
use passage_core::connection::{ConnRef, DispatchError};
use passage_core::packet::{configuration, handshake, login, status};
use passage_core::wire::ByteString;
use passage_core::{Phase, versions};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;
use tracing::{debug, info, warn};

/// Fails unless the connection is at the step this handler answers.
macro_rules! expect_step {
    // Match a pattern and return an expression built from its bindings
    ($conn:expr, $pat:pat => $out:expr) => {
        match &$conn.state.step {
            $pat => Ok($out),
            other => Err(DispatchError::peer(
                "unexpected_step",
                anyhow::anyhow!("Expected step `{}`, got `{:?}`", stringify!($pat), other),
            )),
        }
    };
    // Just check the variant, no extraction
    ($conn:expr, $pat:pat) => {
        expect_step!($conn, $pat => ())
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
pub(crate) async fn on_open(conn: ConnRef<'_, State>) -> crate::Result<(), DispatchError> {
    let shutdown = conn.with(|c| c.shutdown().clone());
    let deadline = conn.with(|c| c.deadline());

    let reason = tokio::select! {
        result = keep_alives(conn) => return result,
        () = shutdown.cancelled() => "disconnect_restart",
        () = expiring(deadline) => "disconnect_timeout",
    };
    farewell(conn, reason).await
}

/// Exchanges keep alives while the client waits for its target.
async fn keep_alives(conn: ConnRef<'_, State>) -> crate::Result<(), DispatchError> {
    loop {
        tokio::time::sleep(KEEP_ALIVE_INTERVAL).await;

        // Keep alives are only exchanged while the client is waiting for its target.
        let (phase, pending) = conn.with(|c| (c.phase(), c.state.keep_alive_id));
        if phase != Phase::Configuration {
            continue;
        }

        // A keep alive that was never answered means the client is gone.
        if pending.is_some() {
            return farewell(conn, "disconnect_timeout").await;
        }

        // Send the next keep alive packet, and remember what the client has to answer with.
        let id = crypto::generate_keep_alive();
        conn.with(|c| {
            c.send(configuration::ServerKeepAlivePacket::new(id))?;
            c.state.keep_alive_id = Some(id);
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
async fn farewell(
    conn: ConnRef<'_, State>,
    reason: &'static str,
) -> crate::Result<(), DispatchError> {
    let phase = conn.phase();
    if !matches!(phase, Phase::Login | Phase::Configuration) {
        return Err(DispatchError::peer(reason, anyhow!("{reason}")));
    }

    let message = conn.localize(reason, &[]).await;
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

pub(crate) async fn on_handshake_intention_packet(
    conn: ConnRef<'_, State>,
    packet: handshake::ClientIntentionPacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::Intention)?;
        c.state.step = packet.next_state.into();

        // Update the connection state.
        c.set_version(packet.protocol_version);
        c.set_phase(packet.next_state.into());

        // Update the custom state.
        c.state.find_route(&packet.server_address);
        c.state.client.protocol_version = packet.protocol_version;
        c.state.client.server_address = packet.server_address;
        c.state.client.server_port = packet.server_port;
        Ok(())
    })
}

pub(crate) async fn on_status_request_packet(
    conn: ConnRef<'_, State>,
    _packet: status::ClientStatusRequestPacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::StatusRequest)?;
        c.state.step = Step::StatusPingRequest;
        Ok::<(), DispatchError>(())
    })?;

    // Query the status adapter based on the selected route. Then, send the status response to the
    // client. Any adapter errors are handled directly.
    let status = match conn.status().await {
        Ok(status) => status.unwrap_or_default(),
        Err(err) => {
            if !err.is_rejected() {
                warn!(err = %err, "status adapter error");
            }
            conn.close();
            return Ok(());
        }
    };

    // Send the status response to the client.
    let packet = status::ServerStatusResponsePacket::try_from(&status)
        .map_err(|err| DispatchError::internal("status_encode_error", err))?;
    conn.send(packet)?;
    Ok(())
}

pub(crate) async fn on_status_ping_request_packet(
    conn: ConnRef<'_, State>,
    packet: status::ClientPingRequestPacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        // Ensure that the connection is in the right state.
        expect_step!(c, Step::Intention)?;
        c.state.step = Step::Completed;

        // Send the pong response and close the connection.
        c.send(status::ServerPongResponsePacket {
            payload: packet.payload,
        })?;
        c.close();
        Ok(())
    })
}

pub(crate) async fn on_login_login_start(
    conn: ConnRef<'_, State>,
    packet: login::ClientLoginStartPacket,
) -> crate::Result<(), DispatchError> {
    let (transfer, client_address, cookie_expiry) = conn.with(|c| {
        // Ensure that the connection is in the right state.
        let transfer = *expect_step!(c, Step::LoginStart { transfer } => transfer)?;

        // Update the custom state.
        c.state.player.name = packet.name.to_string();
        c.state.player.id = packet.uuid;

        let client_address = c.state.client.address;
        let auth_cookie_expiry = c.state.auth_cookie_expiry;
        Ok::<_, DispatchError>((transfer, client_address, auth_cookie_expiry))
    })?;

    // Ensure that the client supports the transfer packet. Otherwise, close the connection with a
    // disconnect packet. The packet will tell the client which minecraft version they should use.
    if conn.version() <= versions::V1_20_5 {
        let preferred = conn
            .status()
            .await
            .unwrap_or_default()
            .unwrap_or_default()
            .version
            .name;
        let reason = conn
            .localize("disconnect_unsupported", &[("preferred", preferred)])
            .await;
        conn.send(login::ServerDisconnectPacket { reason })?;
        conn.close();
        return Ok(());
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
                debug!("invalid auth cookie payload received, skipping auth cookie");
                break 'transfer;
            }

            // Update the auth state
            authenticated = true;
            conn.with(|c| {
                c.state.player.name = auth_cookie.user_name.to_string();
                c.state.player.id = auth_cookie.user_id;
                c.state.player.profile_properties = auth_cookie.profile_properties;
            })
        }
    }

    // Send the encryption message.
    let verify_token = crypto::generate_token()
        .map_err(|err| DispatchError::internal("verify_token_error", err))?;
    conn.with(|c| {
        c.send(login::ServerEncryptionRequestPacket {
            server_id: ByteString::new(),
            public_key: crypto::ENCODED_PUB.clone(),
            verify_token: Default::default(),
            should_authenticate: !authenticated,
        })?;
        c.state.step = Step::Encrypt {
            verify_token,
            authenticated,
        };
        Ok::<_, DispatchError>(())
    })?;
    Ok(())
}

pub(crate) async fn on_login_encryption_response(
    conn: ConnRef<'_, State>,
    packet: login::ClientEncryptionResponsePacket,
) -> crate::Result<(), DispatchError> {
    // Ensure that the connection is in the right state. The step contains the generated verify-token
    // and the authentication status.
    let (verify_token, authenticated) = conn.with(|c| {
        expect_step!(c, Step::Encrypt { verify_token, authenticated } => (verify_token.clone(), *authenticated))
    })?;

    // Decrypt the shared secret and the verify-token.
    let shared_secret = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.shared_secret)
        .map_err(|err| DispatchError::internal("shared_secret_decrypt_error", err))?;
    let decrypted_verify_token = crypto::decrypt(&crypto::KEY_PAIR.0, &packet.verify_token)
        .map_err(|err| DispatchError::internal("verify_token_decrypt_error", err))?;

    // Encrypt the connection either way. Event if the secret is invalid, the client expects it to
    // be encrypted with the secret key.
    conn.with(|c| c.encrypt(Box::new(NoCipher)));

    // Verify the shared secret against the keypair. If the secret is invalid, then we try to send
    // a disconnect packet. It is possible that the client cannot decrypt it, but that's still better
    // than just closing the connection.
    debug!("verifying verify token");
    if !crypto::verify_token(&verify_token, &decrypted_verify_token) {
        let message = conn.localize("disconnect_invalid_session", &[]).await;
        info!("received invalid verify token, closing connection");
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
                if !err.is_rejected() {
                    warn!(err = %err, "profile adapter error");
                }
                let reason = err.reason().unwrap_or("disconnect_error");
                let message = conn.localize(reason, &[]).await;
                return conn.with(|c| {
                    c.state.step = Step::Completed;
                    c.send(login::ServerDisconnectPacket::text(message))?;
                    c.close();
                    Ok(())
                });
            }
        };
        conn.with(|c| {
            c.state.player.name = profile.name.to_string();
            c.state.player.id = profile.id;
            c.state.player.profile_properties = profile.properties;
        });
    }

    // Get the session information of the client.
    let session_cookie = conn.cookie::<SessionCookie>().await?;
    // TODO also store session information

    // Complete the login phase and prepare the target selection.
    conn.with(|c| {
        c.send(login::ServerLoginSuccessPacket {
            uuid: c.state.player.id,
            name: c.state.player.name.as_str().into(),
            properties: c.state.player.profile_properties.clone(),
            strict_error_handling: Some(true),
            session_id: session_cookie.as_ref().map(|cookie| cookie.id.clone()),
        })?;
        c.state.step = Step::LoginAck;
        c.release();
        Ok::<_, DispatchError>(())
    })
}

pub(crate) async fn on_login_login_acknowledged(
    conn: ConnRef<'_, State>,
    _: login::ClientLoginAcknowledgedPacket,
) -> crate::Result<(), DispatchError> {
    // Ensure that the connection is in the right state.
    conn.with(|c| {
        expect_step!(c, Step::LoginAck)?;
        c.set_phase(Phase::Configuration);
        c.state.step = Step::Transfer;
        Ok::<(), DispatchError>(())
    })?;

    // Wait for a target to be selected.
    let target = match conn.target().await {
        Ok(target) => target,
        Err(err) => {
            if !err.is_rejected() {
                warn!(err = %err, "target selection error");
            }
            let reason = err.reason().unwrap_or("disconnect_error");
            let message = conn.localize(reason, &[]).await;
            return conn.with(|c| {
                c.state.step = Step::Completed;
                c.send(configuration::ServerDisconnectPacket::text(message))?;
                c.close();
                Ok(())
            });
        }
    };

    // Send the new with cookie if enabled
    conn.with(|c| {
        let Some(secret) = &c.state.secret else {
            return Ok(());
        };
        let cookie = AuthCookie {
            client_addr: c.state.client.address.clone(),
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
        let encoded = match cookie.encode(Some(&secret)) {
            Ok(encoded) => encoded,
            Err(err) => {
                warn!(err = %err, "failed to encode auth cookie, skipping");
                return Ok(());
            }
        };
        c.send(configuration::ServerStoreCookiePacket {
            key: AuthCookie::KEY.try_into().expect("infallible"),
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
        Ok(())
    })
}

pub(crate) async fn on_configuration_client_information(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientClientInformationPacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        expect_step!(c, Step::Transfer)?;
        c.state.locale = Some(packet.locale.to_string());
        Ok(())
    })
}

pub(crate) async fn on_configuration_keep_alive(
    conn: ConnRef<'_, State>,
    packet: configuration::ClientKeepAlivePacket,
) -> crate::Result<(), DispatchError> {
    conn.with(|c| {
        expect_step!(c, Step::Transfer)?;
        if c.state.keep_alive_id == Some(packet.id) {
            c.state.keep_alive_id = None;
        } else {
            debug!(
                sent = ?c.state.keep_alive_id,
                recieved = ?packet.id,
                "received keep alive packet with invalid id",
            );
        }
        Ok(())
    })
}
