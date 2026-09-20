//! What two routers say to each other, and how the conversation ends.
//!
//! Every test here is a server router and a client router meeting over a socket pair. Each one
//! names the property it proves; the [`Scenario`] takes care of everything that is not the point.

mod common;

use common::packets::*;
use common::*;
use passage_core::connection::{Conn, ConnRef, ConnectionError, DispatchError, Options};
use passage_core::router::UnknownPolicy;
use passage_core::{Phase, ProtocolVersion, versions};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The status half of the test protocol: answer the request, answer the ping, then close.
fn status_server() -> passage_core::router::Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|conn, _packet| {
            conn.state.push("status requested");
            conn.send(StatusResponse::text("mc.justchunks.net"))?;
            Ok(())
        })
        .handle::<Ping>(|conn, packet| {
            conn.state.push(format!("ping {}", packet.payload));
            conn.send(Pong {
                payload: packet.payload,
            })?;
            conn.close();
            Ok(())
        })
        .build()
}

/// A client that asks for the status, pings, and closes when the pong arrives.
fn status_client() -> passage_core::router::Router<Notes> {
    router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Status)?;
            conn.send(StatusRequest)?;
            Ok(())
        }))
        .handle::<StatusResponse>(|conn, packet| {
            conn.state.push(format!("status {}", packet.body));
            conn.send(Ping { payload: 0x1234 })?;
            Ok(())
        })
        .note_and_close::<Pong>()
        .build()
}

#[tokio::test]
async fn a_status_ping_is_served_end_to_end() {
    let meeting = Scenario::new(status_server(), status_client())
        .version(versions::V26_1)
        .run()
        .await;

    meeting.expect_clean();
    assert_eq!(
        meeting.server_saw(),
        [
            "handshake mc.justchunks.net Status",
            "status requested",
            "ping 4660",
        ],
    );
    assert_eq!(
        meeting.client_saw(),
        ["status mc.justchunks.net", "Pong { payload: 4660 }"],
    );
}

#[tokio::test]
async fn a_version_nothing_can_place_is_answered_rather_than_failing_internally() {
    // A client is free to send a negative version, and one that does still has to be *answerable*:
    // dispatch places anything it cannot order on the floor table, so the request arrives -- and the
    // encoder has to agree, or answering ordinary peer input would be an internal error of ours.
    // Status is exactly what this connection is for: it is how the client learns what to install.
    for version in [
        ProtocolVersion::new(-1),
        ProtocolVersion::new(0x4000_0000 | 132),
        ProtocolVersion::new(765),
    ] {
        let meeting = Scenario::new(status_server(), status_client())
            .version(version)
            .run()
            .await;

        meeting.expect_clean();
        assert!(
            meeting
                .client_saw()
                .contains(&"status mc.justchunks.net".to_owned()),
            "{version} was not answered: {:?}",
            meeting.client_saw(),
        );
    }
}

#[tokio::test]
async fn a_release_nobody_wrote_down_is_dispatched_like_the_one_below_it() {
    // Protocol 768 is 1.21.2: a real release, and one no list in this crate mentions. Tables are
    // built from the versions the *packets* name, so 768 is dispatched exactly like 767 -- which is
    // what the protocol says, since nothing changed between them.
    let meeting = Scenario::new(login_server(), login_client())
        .version(ProtocolVersion::new(768))
        .run()
        .await;

    meeting.expect_clean();
    // Below the 26.1 threshold, so the gated field is not on the wire.
    assert_eq!(
        meeting.client_saw(),
        [r#"LoginSuccess { user_name: "Hydrofin", session_id: None }"#],
    );
}

/// Shuts the gate, waits long enough for the peer to speak out of turn, and opens it again.
///
/// Written as a named `async fn` rather than a closure: a closure whose future captures the
/// connection cannot be inferred as higher-ranked over its lifetime, which is the one piece of
/// friction in this design. Real handlers are named functions anyway.
async fn gated_for_a_moment(
    conn: ConnRef<'_, Notes>,
    _packet: LoginStart,
) -> Result<(), DispatchError> {
    conn.with(Conn::gate);
    tokio::time::sleep(Duration::from_millis(50)).await;
    conn.with(Conn::release);
    Ok(())
}

/// The same, but long enough that the peer hangs up first.
async fn gated_for_a_while(
    conn: ConnRef<'_, Notes>,
    _packet: LoginStart,
) -> Result<(), DispatchError> {
    conn.with(Conn::gate);
    tokio::time::sleep(Duration::from_secs(30)).await;
    conn.with(Conn::release);
    Ok(())
}

/// Answers the login, then takes its time finding somewhere to send the player.
///
/// The gate is never shut, so this is the handler that has to overlap with further traffic: the
/// loop keeps reading frames and firing ticks for the whole forty seconds it waits.
async fn slow_transfer(conn: ConnRef<'_, Notes>, packet: LoginStart) -> Result<(), DispatchError> {
    // Answering and moving both sides on happens before the first await, so the client is already
    // in the configuration phase by the time the next frame is routed.
    conn.with(|c| {
        c.send(LoginSuccess {
            user_name: packet.user_name,
            session_id: None,
        })?;
        c.set_phase(Phase::Configuration);
        Ok::<_, ConnectionError>(())
    })?;

    tokio::time::sleep(Duration::from_secs(40)).await;

    // One closure, so a keep-alive cannot land between the transfer and the close.
    conn.with(|c| {
        c.send(Transfer {
            host: "backend-1".into(),
            port: 25_565,
        })?;
        c.close();
        Ok::<_, ConnectionError>(())
    })?;
    Ok(())
}

/// A server that accepts a login and answers it.
fn login_server() -> passage_core::router::Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, packet| {
            conn.state.push(format!("login {}", packet.user_name));
            conn.send(LoginSuccess {
                user_name: packet.user_name,
                session_id: Some(uuid::Uuid::from_u128(7)),
            })?;
            conn.close();
            Ok(())
        })
        .build()
}

/// A client that logs in and closes when it is told it worked.
fn login_client() -> passage_core::router::Router<Notes> {
    router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            Ok(())
        }))
        .note_and_close::<LoginSuccess>()
        .build()
}

#[tokio::test]
async fn a_field_that_only_exists_above_a_threshold_follows_the_peers_version() {
    // A new client must be sent the session id...
    let meeting = Scenario::new(login_server(), login_client())
        .version(versions::V26_1)
        .run()
        .await;
    meeting.expect_clean();
    assert!(
        meeting.client_saw()[0].contains("session_id: Some"),
        "{:?}",
        meeting.client_saw(),
    );

    // ...and an old one must not be, because it would not survive reading it.
    let meeting = Scenario::new(login_server(), login_client())
        .version(versions::V1_20_5)
        .run()
        .await;
    meeting.expect_clean();
    assert!(
        meeting.client_saw()[0].contains("session_id: None"),
        "{:?}",
        meeting.client_saw(),
    );
}

#[tokio::test]
async fn a_packet_that_does_not_exist_in_the_peers_version_is_refused() {
    // `Transfer` does not exist before 1.20.5. Sending it anyway would leave the peer waiting
    // forever for something we never sent, so it is our error rather than a silent no-op.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, _packet| {
            conn.send(Transfer {
                host: "backend-1".into(),
                port: 25_565,
            })?;
            Ok(())
        })
        .build();

    let meeting = Scenario::new(server, login_client())
        .version(ProtocolVersion::new(765))
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("codec"));
    assert!(
        !meeting.server_blamed_peer(),
        "sending something that does not exist is ours, not the peer's",
    );
}

#[tokio::test]
async fn a_handler_that_re_pins_the_version_answers_in_it() {
    // There is no such thing as a stale encoding any more. A handler holds the connection while it
    // writes, so the version it just set is the version the next packet is encoded against -- there
    // is no window in which the two could disagree.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, packet| {
            conn.set_version(versions::V26_1);
            conn.send(LoginSuccess {
                user_name: packet.user_name,
                session_id: Some(uuid::Uuid::from_u128(7)),
            })?;
            conn.close();
            Ok(())
        })
        .build();

    // The client reads at 26.1 too, so it can see the field that only exists there. If the packet
    // had gone out under the version the handler *started* in, this would not decode.
    let meeting = Scenario::new(server, login_client())
        .version(versions::V26_1)
        .run()
        .await;

    meeting.expect_clean();
    assert!(
        meeting.client_saw()[0].contains("session_id: Some"),
        "{:?}",
        meeting.client_saw(),
    );
}

#[tokio::test]
async fn what_a_handler_queued_before_it_failed_is_discarded() {
    // A handler that answers an error itself must not also fail with one. `on_error` gets a fresh
    // queue precisely so that nothing a failing handler left behind can interleave with the last
    // word -- so "send this, then fail" sends nothing, and the hook below is what the peer hears.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, _packet| {
            conn.send(Disconnect::text("go away"))?;
            Err(DispatchError::peer("refused", anyhow::anyhow!("not today")))
        })
        .on_error(errors(|conn, error| {
            conn.send(Disconnect::text(error.reason()))?;
            conn.close();
            Ok(())
        }))
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            Ok(())
        }))
        .note_and_close::<Disconnect>()
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .run()
        .await;

    assert_eq!(
        meeting.client_saw(),
        [r#"Disconnect { reason: "refused" }"#],
        "the hook's answer, not the one the failing handler queued",
    );
    assert_eq!(meeting.server_ending(), Some("refused"));
    assert!(meeting.server_blamed_peer());
}

#[tokio::test]
async fn a_handler_that_answers_an_error_itself_closes_rather_than_failing() {
    // The other side of the decision above: a handler *can* have the last word, by saying it and
    // then ending the connection instead of raising. Everything queued before the close is written.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, _packet| {
            conn.state.push("refusing");
            conn.send(Disconnect::text("go away"))?;
            conn.close();
            Ok(())
        })
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            Ok(())
        }))
        .note_and_close::<Disconnect>()
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .run()
        .await;

    meeting.expect_clean();
    assert_eq!(
        meeting.client_saw(),
        [r#"Disconnect { reason: "go away" }"#]
    );
}

#[tokio::test]
async fn the_error_hook_gets_the_last_word() {
    // Every ending that is not a handler closing the connection arrives here, which is where a
    // disconnect message comes from. It is a last word, not a veto: the connection ends either way.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .on_error(errors(|conn, error| {
            conn.state.push(format!("ending {}", error.reason()));
            if error.can_reply() {
                conn.send(Disconnect::text("we are restarting"))?;
                conn.close();
            }
            Ok(())
        }))
        .build();
    let client = router()
        .on_open(opening(Intent::Login))
        .note_and_close::<Disconnect>()
        .build();

    let shutdown = CancellationToken::new();
    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .server_config(|config| config.max_lifetime = Some(Duration::from_millis(50)))
        .shutdown(shutdown)
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("peer-timeout"));
    assert_eq!(
        meeting.server_saw(),
        ["handshake mc.justchunks.net Login", "ending peer-timeout"]
    );
    assert_eq!(
        meeting.client_saw(),
        [r#"Disconnect { reason: "we are restarting" }"#],
        "the peer was told why, rather than seeing a bare socket close",
    );
}

#[tokio::test]
async fn an_answer_that_cannot_be_sent_does_not_replace_the_reason() {
    // If the last word fails -- and the commonest way is a packet that does not exist in a version
    // the handshake never pinned -- the connection still reports what it was ending for. Reporting
    // the failed apology instead would lose the diagnosis exactly when it is most wanted.
    let server = router()
        .handle::<Handshake>(|_conn, _packet: Handshake| {
            Err(DispatchError::peer(
                "the_real_reason",
                anyhow::anyhow!("what actually went wrong"),
            ))
        })
        .on_error(errors(|conn, _error| {
            // `Transfer` does not exist at the version this connection never got past.
            conn.send(Transfer {
                host: "nowhere".into(),
                port: 1,
            })?;
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, router().on_open(opening(Intent::Login)).build())
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("the_real_reason"));
}

#[tokio::test]
async fn a_hangup_is_an_ending_but_not_a_failure_worth_reporting() {
    // Nothing here is anyone's fault, and a scanner that takes its status and leaves must never be
    // logged as a problem -- which is why blame is a separate question from whether it failed.
    let server = router().handle::<Handshake>(on_handshake).build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Status)?;
            conn.close();
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, client).run().await;

    assert_eq!(meeting.client_ending(), None, "the client meant to leave");
    assert_eq!(meeting.server_ending(), Some("peer-closed"));
    let error = meeting.server.error.expect("an ending");
    assert!(error.is_peer_error(), "ordinary weather, not a problem");
    assert!(!error.can_reply(), "there is nobody left to read an answer");
}

#[tokio::test]
async fn state_is_recorded_before_the_packet_that_announces_it() {
    // The state is written where the handler writes it, and the packet only leaves once the round
    // is drained -- so by the time the peer has the packet, the update is long since applied. That
    // is what a tick or a later handler would observe.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|conn, packet| {
            conn.state.push(format!("recorded {}", packet.user_name));
            conn.send(LoginSuccess {
                user_name: packet.user_name,
                session_id: None,
            })?;
            conn.close();
            Ok(())
        })
        .build();

    let meeting = Scenario::new(server, login_client())
        .version(versions::V26_1)
        .run()
        .await;

    meeting.expect_clean();
    assert_eq!(
        meeting.server_saw(),
        ["handshake mc.justchunks.net Login", "recorded Hydrofin"],
    );
}

#[tokio::test]
async fn a_packet_nobody_routes_ends_the_connection_as_a_peer_error() {
    let server = router().handle::<Handshake>(on_handshake).build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Status)?;
            // Registered by nobody in the status phase.
            conn.send(Ping { payload: 1 })?;
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("unknown_packet"));
    assert!(meeting.server_blamed_peer());
}

#[tokio::test]
async fn a_minimal_driver_can_ignore_what_it_does_not_route() {
    // A packet nobody registered is not automatically fatal: which it is, is the router's to say.
    let server = router()
        .unknown(UnknownPolicy::Ignore)
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|conn, _packet| {
            conn.state.push("still here");
            conn.close();
            Ok(())
        })
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Status)?;
            conn.send(Ping { payload: 1 })?;
            conn.send(StatusRequest)?;
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .run()
        .await;

    assert_eq!(
        meeting.server_saw(),
        ["handshake mc.justchunks.net Status", "still here"]
    );
}

#[tokio::test]
async fn a_packet_sent_during_an_exclusive_task_is_a_protocol_error() {
    // Both packets are written before the server has read the first. The login handler is
    // exclusive, so the acknowledgement was sent before the client could possibly have seen an
    // answer -- it is a protocol break, and reporting it is the whole point of the gate.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle_async(gated_for_a_moment)
        .handle::<LoginAcknowledged>(|conn, _packet| {
            conn.state.push("acknowledged");
            Ok(())
        })
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            conn.send(LoginAcknowledged)?;
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("early-packet"));
    assert!(meeting.server_blamed_peer());
    assert!(
        !meeting.server_saw().contains(&"acknowledged".to_owned()),
        "the packet must not be dispatched into a half-finished login",
    );
}

#[tokio::test]
async fn a_hangup_during_an_exclusive_task_ends_the_connection_at_once() {
    // The socket is still polled while the gate is shut, so a client that disappears mid-login is
    // noticed immediately instead of after the slow call returns.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle_async(gated_for_a_while)
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            conn.close();
            Ok(())
        }))
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .timeout(Duration::from_secs(5))
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("peer-closed"));
}

#[tokio::test(start_paused = true)]
async fn spawned_work_runs_while_keep_alives_are_exchanged() {
    // Work that must overlap with further traffic is spawned rather than exclusive, so the gate
    // stays open and the tick handler keeps the connection alive while it runs.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle_async(slow_transfer)
        .handle::<KeepAliveResponse>(|conn, packet| {
            conn.state.push(format!("alive {}", packet.id));
            Ok(())
        })
        .on_tick(ticks(|conn| {
            // Only once the connection has something to wait for.
            if conn.phase() == Phase::Configuration {
                conn.send(KeepAlive { id: 1 })?;
            }
            Ok(())
        }))
        .build();
    let client = router()
        .on_open(opens(|conn| {
            greet(conn, Intent::Login)?;
            conn.send(LoginStart::named("Hydrofin"))?;
            Ok(())
        }))
        .handle::<LoginSuccess>(|conn, _packet| {
            conn.set_phase(Phase::Configuration);
            Ok(())
        })
        .handle::<KeepAlive>(|conn, packet| {
            conn.send(KeepAliveResponse { id: packet.id })?;
            Ok(())
        })
        .note_and_close::<Transfer>()
        .build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .server_config(|config| {
            config.tick_interval = Some(Duration::from_secs(16));
            config.max_lifetime = Some(Duration::from_secs(120));
        })
        .run()
        .await;

    meeting.expect_clean();
    assert_eq!(
        meeting.client_saw(),
        [r#"Transfer { host: "backend-1", port: 25565 }"#],
    );
    assert!(
        meeting
            .server_saw()
            .iter()
            .filter(|line| line.starts_with("alive"))
            .count()
            >= 2,
        "the connection was kept alive while the work ran: {:?}",
        meeting.server_saw(),
    );
}

#[tokio::test(start_paused = true)]
async fn a_connection_nobody_configured_still_has_a_deadline() {
    // The documented minimal setup is the default, so that is where the security posture lives: a
    // peer that connects and then says nothing must not hold the socket forever.
    let server = router().handle::<Handshake>(on_handshake).build();
    let client = router().on_open(opening(Intent::Login)).build();

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .client_config(|config| config.max_lifetime = Some(Duration::from_secs(3600)))
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("peer-timeout"));
    assert_eq!(
        Options::default().max_lifetime,
        Some(Duration::from_secs(120))
    );
}

#[tokio::test]
async fn cancelling_the_shutdown_token_ends_both_sides() {
    let shutdown = CancellationToken::new();
    let server = router()
        .handle::<Handshake>(|conn, packet| {
            on_handshake(conn, packet)?;
            // Everything is up and running, and then the operator restarts the server.
            Ok(())
        })
        .build();
    let client = router().on_open(opening(Intent::Login)).build();

    let cancelling = shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancelling.cancel();
    });

    let meeting = Scenario::new(server, client)
        .version(versions::V26_1)
        .shutdown(shutdown)
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("shutdown"));
    assert_eq!(meeting.client_ending(), Some("shutdown"));
}
