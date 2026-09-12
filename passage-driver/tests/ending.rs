//! Tests for how a connection ends, and for the last word the dispatcher gets.
//!
//! The property under test throughout is that **nothing a handler queued is lost**. Before
//! [`Dispatcher::on_error`] existed, returning an error skipped straight past the operation queue,
//! so "send this disconnect, then fail" silently sent nothing -- and a shutdown, a deadline or a
//! decoding bug all reached the client as a bare socket close.

mod common;

use common::{TestClient, intention};
use passage_driver::conn::{Connection, ConnectionConfig, Ctx, Dispatcher, Ending, Outcome};
use passage_driver::demo::packets::{
    Intent, Intention, LoginDisconnect, LoginStart, StatusRequest,
};
use passage_driver::demo::server::{Session, router};
use passage_driver::error::{Class, Error, Result};
use passage_driver::packet::Phase;
use passage_driver::router::{Router, RouterDispatcher, UnknownPolicy};
use passage_driver::version::{ProtocolVersion, versions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

fn connect_to<D: Dispatcher<Session> + Send + 'static>(
    dispatcher: D,
    config: ConnectionConfig,
    shutdown: CancellationToken,
) -> (TestClient, JoinHandle<Outcome<Session>>) {
    let (server_io, client_io) = tokio::io::duplex(4096);
    let (connection, _handle) = Connection::builder(server_io, dispatcher, Session::default())
        .config(config)
        .shutdown(shutdown)
        .build();
    (TestClient::new(client_io), tokio::spawn(connection.run()))
}

fn demo() -> RouterDispatcher<Session> {
    RouterDispatcher::new(router().expect("the demo router is well-formed"))
}

/// Walks a client up to the login phase, where the demo has a disconnect packet to send.
async fn reach_login(client: &mut TestClient) {
    client
        .send(&intention(versions::V26_2, Intent::Login))
        .await;
    client.version = versions::V26_2;
}

#[tokio::test]
async fn a_refused_login_is_told_why() {
    // 1.20.4: below the configuration phase, so it cannot be served. The old implementation sent a
    // localized disconnect here; the first version of this driver could only close the socket.
    let (mut client, server) = connect_to(
        demo(),
        ConnectionConfig::default(),
        CancellationToken::new(),
    );
    client
        .send(&intention(ProtocolVersion::new(765), Intent::Login))
        .await;
    client.version = ProtocolVersion::new(765);

    // The handshake handler queued `set_phase(Login)` *before* it failed, and operations drain
    // before `on_error` runs -- which is what puts the connection in a phase that has a disconnect
    // packet at all.
    let disconnect = client.expect::<LoginDisconnect>().await;
    assert!(
        disconnect.reason.contains("could not use"),
        "{disconnect:?}"
    );
    client.expect_eof().await;

    let outcome = server.await.expect("no panic");
    assert_eq!(
        outcome.result.expect_err("refused").label(),
        "unsupported_version"
    );
    // The ending is still the reason the connection ended. `on_error` had the last word on the
    // wire, not on the report.
    assert_eq!(outcome.phase, Phase::Login);
    // And "this one was a refusal" is a fact about the session, recorded by the handler that knew
    // it -- with a reason attached, which is what a `Completion` variant could not have carried.
    assert_eq!(outcome.state.refused, Some("bad_client"));
}

#[tokio::test]
async fn a_shutdown_gets_a_message_out_even_though_it_cancelled_the_connection() {
    // The token that ends the connection is the same one that would otherwise abort the write, so
    // the final stretch is bounded by `close_timeout` instead. Without that, the one ending most
    // worth explaining -- "we are restarting" -- is the one that could never be explained.
    let shutdown = CancellationToken::new();
    let (mut client, server) = connect_to(demo(), ConnectionConfig::default(), shutdown.clone());

    reach_login(&mut client).await;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;
    let _ = client
        .expect::<passage_driver::demo::packets::LoginSuccess>()
        .await;

    shutdown.cancel();

    let disconnect = client.expect::<LoginDisconnect>().await;
    assert!(disconnect.reason.contains("restarting"), "{disconnect:?}");
    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::Cancelled),
    ));
}

#[tokio::test]
async fn what_a_handler_queued_before_it_failed_is_still_written() {
    // The A4 case in its own right: no `on_error` involved, just a handler that says something and
    // then gives up. Both halves have to happen, in that order.
    let router = Router::<Session>::builder()
        .on::<Intention>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Login)
        })
        .on::<LoginStart>(|ctx: Ctx<'_, Session>, _packet: LoginStart| {
            ctx.send(LoginDisconnect::text("go away"))?;
            Err(Error::peer("refused", "not today"))
        })
        .build()
        .expect("builds");

    let (mut client, server) = connect_to(
        RouterDispatcher::new(router),
        ConnectionConfig::default(),
        CancellationToken::new(),
    );
    reach_login(&mut client).await;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;

    assert_eq!(
        client.expect::<LoginDisconnect>().await,
        LoginDisconnect::text("go away"),
    );
    assert_eq!(
        server
            .await
            .expect("no panic")
            .result
            .expect_err("refused")
            .label(),
        "refused",
    );
}

#[tokio::test]
async fn a_batch_reaches_the_connection_with_nothing_in_between() {
    // Two ops queued separately can be split by anything else holding a handle. A batch cannot.
    // The disconnect handler depends on this: a keep-alive landing between the message and the
    // close would be written after the peer had already been told to go.
    let router = Router::<Session>::builder()
        .on::<Intention>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Login)
        })
        .on::<LoginStart>(|ctx: Ctx<'_, Session>, _packet: LoginStart| {
            ctx.batch(|batch| {
                batch.send(LoginDisconnect::text("first"))?;
                batch.send(LoginDisconnect::text("second"))?;
                batch.close();
                Ok(())
            })
        })
        .build()
        .expect("builds");

    let (mut client, server) = connect_to(
        RouterDispatcher::new(router),
        ConnectionConfig::default(),
        CancellationToken::new(),
    );
    reach_login(&mut client).await;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;

    assert_eq!(
        client.expect::<LoginDisconnect>().await,
        LoginDisconnect::text("first"),
    );
    assert_eq!(
        client.expect::<LoginDisconnect>().await,
        LoginDisconnect::text("second"),
    );
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn a_batch_that_cannot_be_built_queues_nothing() {
    // `Transfer` does not exist before 1.20.5, so encoding it for an unknown version fails. All or
    // nothing means the packet queued before it does not go out either -- which is what lets
    // `on_error` try to speak without risking a half-sent answer.
    let router = Router::<Session>::builder()
        .on::<StatusRequest>(|ctx: Ctx<'_, Session>, _packet: StatusRequest| {
            let attempted = ctx.batch(|batch| {
                batch.send(StatusRequest)?;
                batch.send(passage_driver::demo::packets::Transfer {
                    host: "nowhere".to_owned(),
                    port: 1,
                })?;
                Ok(())
            });
            assert!(attempted.is_err(), "the batch must not build");
            ctx.close()
        })
        .build()
        .expect("builds");

    let (mut client, server) = connect_to(
        RouterDispatcher::new(router),
        ConnectionConfig {
            initial_phase: Phase::Status,
            ..ConnectionConfig::default()
        },
        CancellationToken::new(),
    );
    // Straight into the status phase, at the version nothing resolves for.
    client.send(&StatusRequest).await;
    client.expect_eof().await;

    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn a_handler_decides_for_itself_what_is_survivable() {
    // There is no "recover from this error" hook, and there should not be: by the time a connection
    // is ending, the packet that went wrong and the state around it are gone. The handler that saw
    // both is the one that gets to say an error is survivable, and it says it by not raising one.
    let router = Router::<Session>::builder()
        // The router-wide half of the same idea: a packet nobody routes is not automatically fatal.
        .unknown(UnknownPolicy::Ignore)
        .on::<Intention>(|ctx: Ctx<'_, Session>, packet: Intention| {
            ctx.set_version(packet.protocol_version)?;
            ctx.set_phase(Phase::Login)
        })
        .on::<LoginStart>(|ctx: Ctx<'_, Session>, packet: LoginStart| {
            // A name we do not like. Refusing would be reasonable; carrying on is the handler's
            // call to make, and it is the only place with enough context to make it.
            if packet.user_name.is_empty() {
                return Ok(());
            }
            ctx.send(LoginDisconnect::text("welcome"))?;
            ctx.close()
        })
        .build()
        .expect("builds");

    let (mut client, server) = connect_to(
        RouterDispatcher::new(router),
        ConnectionConfig::default(),
        CancellationToken::new(),
    );
    reach_login(&mut client).await;

    // An unroutable id, then the packet the handler waves off, then a real one.
    client.send_raw(&[0x7F]).await;
    client
        .send(&LoginStart {
            user_name: String::new(),
            user_id: Uuid::nil(),
        })
        .await;
    client
        .send(&LoginStart {
            user_name: "Hydrofin".to_owned(),
            user_id: Uuid::nil(),
        })
        .await;

    assert_eq!(
        client.expect::<LoginDisconnect>().await,
        LoginDisconnect::text("welcome"),
    );
    server.await.expect("no panic").result.expect("no error");
}

#[tokio::test]
async fn an_answer_that_cannot_be_sent_does_not_replace_the_reason() {
    // `on_error` is a last word, not a second cause. If it fails -- and the commonest way is a
    // packet that does not exist in a version the handshake never pinned -- the connection still
    // reports what it was ending for. Reporting the failed apology instead would lose the diagnosis
    // exactly when it is most wanted.
    struct Unhelpful;

    impl Dispatcher<Session> for Unhelpful {
        fn set_version(&mut self, _version: ProtocolVersion) {}

        fn dispatch(&self, _ctx: Ctx<'_, Session>, _id: i32, _payload: &[u8]) -> Result<()> {
            Err(Error::peer("the_real_reason", "what actually went wrong"))
        }

        fn tick(&self, _ctx: Ctx<'_, Session>) -> Result<()> {
            Ok(())
        }

        fn ticks(&self) -> bool {
            false
        }

        fn on_error(&self, _ctx: Ctx<'_, Session>, _ending: &Ending) -> Result<()> {
            Err(Error::internal(
                "the_answer_broke",
                "and nobody needs to know",
            ))
        }
    }

    let (mut client, server) = connect_to(
        Unhelpful,
        ConnectionConfig::default(),
        CancellationToken::new(),
    );
    client.send_raw(&[0x00]).await;

    let ending = server.await.expect("no panic").result.expect_err("fails");
    assert_eq!(ending.label(), "the_real_reason");
    assert_eq!(ending.error().expect("a failure").class(), Class::Peer);
}

#[tokio::test]
async fn a_peer_that_vanishes_mid_flight_still_runs_cleanup() {
    // The case that motivated moving `PeerClosed` into `Ending`. A client drops while work it asked
    // for is still running; if that ending never reached `on_error`, whatever the work reserved
    // would be reserved forever, and no handler could have caught it -- the connection is gone, so
    // no packet is ever dispatched again.
    #[derive(Clone, Default)]
    struct Reservations(Arc<Mutex<Vec<&'static str>>>);

    impl Dispatcher<Session> for Reservations {
        fn set_version(&mut self, _version: ProtocolVersion) {}

        fn dispatch(&self, ctx: Ctx<'_, Session>, _id: i32, _payload: &[u8]) -> Result<()> {
            self.0.lock().expect("not poisoned").push("reserved");
            // Long enough that the client is gone before it could ever resolve.
            ctx.spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(())
            })
        }

        fn tick(&self, _ctx: Ctx<'_, Session>) -> Result<()> {
            Ok(())
        }

        fn ticks(&self) -> bool {
            false
        }

        fn on_error(&self, _ctx: Ctx<'_, Session>, ending: &Ending) -> Result<()> {
            self.0.lock().expect("not poisoned").push(ending.label());
            Ok(())
        }
    }

    let reservations = Reservations::default();
    let (mut client, server) = connect_to(
        reservations.clone(),
        ConnectionConfig::default(),
        CancellationToken::new(),
    );

    client.send_raw(&[0x00]).await;
    // Let the frame be dispatched, then vanish.
    tokio::task::yield_now().await;
    drop(client);

    assert!(matches!(
        server.await.expect("no panic").result,
        Err(Ending::PeerClosed),
    ));
    assert_eq!(
        *reservations.0.lock().expect("not poisoned"),
        vec!["reserved", "peer_closed"],
        "the reservation was never released",
    );
}
