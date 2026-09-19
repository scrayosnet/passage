//! What two routers say to each other, and how the conversation ends.
//!
//! Every test here is a server router and a client router meeting over a socket pair. Each one
//! names the property it proves; the [`Scenario`] takes care of everything that is not the point.

mod common;

use common::packets::*;
use common::*;
use passage_core::connection::{DispatchError, Options};
use passage_core::router::UnknownPolicy;
use passage_core::{Phase, ProtocolVersion, versions};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The status half of the test protocol: answer the request, answer the ping, then close.
fn status_server() -> passage_core::router::Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<StatusRequest>(|ctx, _packet| {
            ctx.state.push("status requested");
            ctx.handle
                .send(ctx.version, StatusResponse::text("mc.justchunks.net"))?;
            Ok(())
        })
        .handle::<Ping>(|ctx, packet| {
            ctx.state.push(format!("ping {}", packet.payload));
            ctx.handle.batch(|batch| {
                batch.send(
                    ctx.version,
                    Pong {
                        payload: packet.payload,
                    },
                )?;
                batch.close();
                Ok(())
            })?;
            Ok(())
        })
        .build()
}

/// A client that asks for the status, pings, and closes when the pong arrives.
fn status_client() -> passage_core::router::Router<Notes> {
    router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Status)?;
            ctx.handle.send(ctx.version, StatusRequest)?;
            Ok(())
        })
        .handle::<StatusResponse>(|ctx, packet| {
            ctx.state.push(format!("status {}", packet.body));
            ctx.handle.send(ctx.version, Ping { payload: 0x1234 })?;
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

/// A server that accepts a login and answers it.
fn login_server() -> passage_core::router::Router<Notes> {
    router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|ctx, packet| {
            ctx.state.push(format!("login {}", packet.user_name));
            ctx.handle.batch(|batch| {
                batch.send(
                    ctx.version,
                    LoginSuccess {
                        user_name: packet.user_name,
                        session_id: Some(uuid::Uuid::from_u128(7)),
                    },
                )?;
                batch.close();
                Ok(())
            })?;
            Ok(())
        })
        .build()
}

/// A client that logs in and closes when it is told it worked.
fn login_client() -> passage_core::router::Router<Notes> {
    router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle
                .send(ctx.version, LoginStart::named("Hydrofin"))?;
            Ok(())
        })
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
        .handle::<LoginStart>(|ctx, _packet| {
            ctx.handle.send(
                ctx.version,
                Transfer {
                    host: "backend-1".to_owned(),
                    port: 25_565,
                },
            )?;
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
async fn a_packet_encoded_for_a_superseded_version_is_refused() {
    // A handler that re-pins the version cannot also answer in it: its own view of the version is
    // the snapshot from before the change, and writing those bytes would put an ID on the wire that
    // the peer resolves in another table.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|ctx, packet| {
            ctx.handle.set_version(versions::V26_1)?;
            ctx.handle.send(
                ctx.version,
                LoginSuccess {
                    user_name: packet.user_name,
                    session_id: None,
                },
            )?;
            Ok(())
        })
        .build();

    let meeting = Scenario::new(server, login_client())
        .version(versions::V1_20_5)
        .run()
        .await;

    assert_eq!(meeting.server_ending(), Some("stale-encoding"));
    assert!(
        meeting.server_error().contains("LoginSuccess"),
        "{}",
        meeting.server_error()
    );
}

#[tokio::test]
async fn what_a_handler_queued_before_it_failed_is_discarded() {
    // A handler that answers an error itself must not also fail with one. `on_error` gets a fresh
    // queue precisely so that nothing a failing handler left behind can interleave with the last
    // word -- so "send this, then fail" sends nothing, and the hook below is what the peer hears.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|ctx, _packet| {
            ctx.handle.send(ctx.version, Disconnect::text("go away"))?;
            Err(DispatchError::peer("refused", anyhow::anyhow!("not today")))
        })
        .on_error(|ctx, error| {
            ctx.handle
                .send(ctx.version, Disconnect::text(error.reason()))?;
            ctx.handle.close()?;
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle
                .send(ctx.version, LoginStart::named("Hydrofin"))?;
            Ok(())
        })
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
        .handle::<LoginStart>(|ctx, _packet| {
            ctx.state.push("refusing");
            ctx.handle.batch(|batch| {
                batch.send(ctx.version, Disconnect::text("go away"))?;
                batch.close();
                Ok(())
            })?;
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle
                .send(ctx.version, LoginStart::named("Hydrofin"))?;
            Ok(())
        })
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
        .on_error(|ctx, error| {
            ctx.state.push(format!("ending {}", error.reason()));
            if error.can_reply() {
                ctx.handle
                    .send(ctx.version, Disconnect::text("we are restarting"))?;
                ctx.handle.close()?;
            }
            Ok(())
        })
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
        .handle::<Handshake>(|_ctx, _packet| {
            Err(DispatchError::peer(
                "the_real_reason",
                anyhow::anyhow!("what actually went wrong"),
            ))
        })
        .on_error(|ctx, _error| {
            // `Transfer` does not exist at the version this connection never got past.
            ctx.handle.send(
                ctx.version,
                Transfer {
                    host: "nowhere".to_owned(),
                    port: 1,
                },
            )?;
            Ok(())
        })
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
        .on_open(|ctx| {
            greet(&ctx, Intent::Status)?;
            ctx.handle.close()?;
            Ok(())
        })
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
    // A handler queues the update ahead of the packet, and both are operations -- so by the time
    // the peer has the packet, the connection has already applied the update. That is what a tick
    // or a later handler would observe.
    let server = router()
        .handle::<Handshake>(on_handshake)
        .handle::<LoginStart>(|ctx, packet| {
            let name = packet.user_name.clone();
            ctx.handle.batch(|batch| {
                batch.update(move |notes: &mut Notes| notes.push(format!("recorded {name}")));
                batch.send(
                    ctx.version,
                    LoginSuccess {
                        user_name: packet.user_name,
                        session_id: None,
                    },
                )?;
                batch.close();
                Ok(())
            })?;
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
        .on_open(|ctx| {
            greet(&ctx, Intent::Status)?;
            // Registered by nobody in the status phase.
            ctx.handle.send(ctx.version, Ping { payload: 1 })?;
            Ok(())
        })
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
        .handle::<StatusRequest>(|ctx, _packet| {
            ctx.state.push("still here");
            ctx.handle.close()?;
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Status)?;
            ctx.handle.send(ctx.version, Ping { payload: 1 })?;
            ctx.handle.send(ctx.version, StatusRequest)?;
            Ok(())
        })
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
        .handle::<LoginStart>(|ctx, _packet| {
            ctx.handle.exclusive(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok(())
            })?;
            Ok(())
        })
        .handle::<LoginAcknowledged>(|ctx, _packet| {
            ctx.state.push("acknowledged");
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle
                .send(ctx.version, LoginStart::named("Hydrofin"))?;
            ctx.handle.send(ctx.version, LoginAcknowledged)?;
            Ok(())
        })
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
        .handle::<LoginStart>(|ctx, _packet| {
            ctx.handle.exclusive(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            })?;
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle.batch(|batch| {
                batch.send(ctx.version, LoginStart::named("Hydrofin"))?;
                batch.close();
                Ok(())
            })?;
            Ok(())
        })
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
        .handle::<LoginStart>(|ctx, packet| {
            // Answer the login, move both sides on, and start the slow part in the background.
            ctx.handle.batch(|batch| {
                batch.send(
                    ctx.version,
                    LoginSuccess {
                        user_name: packet.user_name,
                        session_id: None,
                    },
                )?;
                batch.set_phase(Phase::Configuration);
                Ok(())
            })?;

            let handle = ctx.handle.clone();
            let version = ctx.version;
            ctx.handle.spawn(async move {
                tokio::time::sleep(Duration::from_secs(40)).await;
                handle.batch(|batch| {
                    batch.send(
                        version,
                        Transfer {
                            host: "backend-1".to_owned(),
                            port: 25_565,
                        },
                    )?;
                    batch.close();
                    Ok(())
                })
            })?;
            Ok(())
        })
        .handle::<KeepAliveResponse>(|ctx, packet| {
            ctx.state.push(format!("alive {}", packet.id));
            Ok(())
        })
        .on_tick(|ctx| {
            // Only once the connection has something to wait for.
            if ctx.phase == Phase::Configuration {
                ctx.handle.send(ctx.version, KeepAlive { id: 1 })?;
            }
            Ok(())
        })
        .build();
    let client = router()
        .on_open(|ctx| {
            greet(&ctx, Intent::Login)?;
            ctx.handle
                .send(ctx.version, LoginStart::named("Hydrofin"))?;
            Ok(())
        })
        .handle::<LoginSuccess>(|ctx, _packet| {
            ctx.handle.set_phase(Phase::Configuration)?;
            Ok(())
        })
        .handle::<KeepAlive>(|ctx, packet| {
            ctx.handle
                .send(ctx.version, KeepAliveResponse { id: packet.id })?;
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
        .handle::<Handshake>(|ctx, packet| {
            on_handshake(ctx, packet)?;
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
