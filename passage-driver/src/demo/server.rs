//! A worked server on top of the driver: the Passage flow, as handlers.
//!
//! This is the layer the README calls "a basic server implementation on top of the backbone". It
//! owns no I/O and no framing -- only the state machine. Compare it to the 470-line `listen()` it
//! replaces: each step is a function you can read, test and override in isolation, and the
//! sequential parts stay sequential because the driver runs at most one handler at a time.

use crate::conn::Ctx;
use crate::demo::packets::{
    Intention, KeepAlive, KeepAliveResponse, LoginAcknowledged, LoginStart, LoginSuccess,
    PingRequest, PongResponse, Property, StatusRequest, StatusResponse, Transfer,
};
use crate::error::{Error, InternalError, ProtocolError, Result};
use crate::flow::{Flow, Outcome, Update};
use crate::packet::{Direction, Phase};
use crate::router::{Router, UnknownPolicy};
use crate::version::{Feature, ProtocolVersion, versions};
use crate::wire::VarInt;
use std::time::Duration;
use uuid::Uuid;

/// The oldest protocol version that can log in: 1.20.5 introduced the configuration phase, cookies
/// and the transfer packet, all of which Passage depends on.
pub const MIN_LOGIN_VERSION: ProtocolVersion = versions::V1_20_5;

/// What the client said it wanted in the handshake.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Intent {
    /// Server list ping.
    #[default]
    Status,
    /// A fresh login.
    Login,
    /// A login continuing from another server's transfer.
    Transfer,
}

/// Per-connection state.
///
/// Plain data: no locks, no atomics, no `Arc`. Handlers get `&mut` to it, and asynchronous work
/// hands changes back through [`Update`].
#[derive(Debug, Default)]
pub struct Session {
    /// The hostname the client connected to.
    pub host: String,
    /// What the client asked for.
    pub intent: Intent,
    /// The name the client claimed in `LoginStart`.
    pub claimed_name: String,
    /// The profile as verified by the authentication adapter.
    pub profile: Option<(String, Uuid)>,
    /// The keep-alive we are waiting for an answer to.
    pub awaiting_keep_alive: Option<i64>,
    /// How many keep-alives went unanswered.
    pub keep_alive_misses: u32,
    /// The next keep-alive id, so the demo stays deterministic.
    pub next_keep_alive: i64,
}

/// Builds the server router.
///
/// This is the whole protocol surface of the server, in one screen: which packets it accepts, in
/// which phase, and what runs for each. Adding a packet is one line here plus one `packet!`
/// declaration -- no trait to widen, no `match` arm to forget.
pub fn router() -> Router<Session> {
    Router::new(Direction::Serverbound)
        .unknown(UnknownPolicy::Reject)
        .on::<Intention, _>(on_intention)
        .on::<StatusRequest, _>(on_status_request)
        .on::<PingRequest, _>(on_ping_request)
        .on::<LoginStart, _>(on_login_start)
        .on::<LoginAcknowledged, _>(on_login_acknowledged)
        .on::<KeepAliveResponse, _>(on_keep_alive_response)
        .on_tick(on_tick)
}

/// The handshake: pick a phase, pin the version, reject what we cannot serve.
fn on_intention(ctx: Ctx<'_, Session>, packet: Intention) -> Outcome<Session> {
    let version = ProtocolVersion::new(packet.protocol_version.0);

    // A negative version is not a version. Refuse it here rather than letting every later ID lookup
    // silently miss.
    if version < ProtocolVersion::UNKNOWN {
        return Flow::fail(ProtocolError::UnsupportedVersion { version });
    }

    let intent = match packet.intent.0 {
        1 => Intent::Status,
        2 => Intent::Login,
        3 => Intent::Transfer,
        _ => {
            return Flow::fail(ProtocolError::UnexpectedPacket {
                packet: "Intention",
                phase: Phase::Handshake,
            });
        }
    };

    // Status has to work for every version -- that is how old clients get told which version to
    // use. Logging in does not.
    if intent != Intent::Status && !version.at_least(MIN_LOGIN_VERSION) {
        return Flow::fail(ProtocolError::UnsupportedVersion { version });
    }

    ctx.conn.set_version(version);
    ctx.conn.set_phase(match intent {
        Intent::Status => Phase::Status,
        Intent::Login | Intent::Transfer => Phase::Login,
    });

    let host = packet.server_address;
    Flow::update(move |session: &mut Session| {
        session.host = host;
        session.intent = intent;
    })
}

/// The status response. Synchronous: no adapter, no allocation beyond the body.
fn on_status_request(ctx: Ctx<'_, Session>, _packet: StatusRequest) -> Outcome<Session> {
    let body = format!(
        r#"{{"version":{{"name":"1.21","protocol":{}}},"players":{{"max":0,"online":0}},"description":{{"text":"{}"}}}}"#,
        ctx.version().get(),
        ctx.state.host,
    );
    Flow::from_result(ctx.send(&StatusResponse { body }))
}

/// The latency probe, and the end of a status connection.
fn on_ping_request(ctx: Ctx<'_, Session>, packet: PingRequest) -> Outcome<Session> {
    Flow::from_result(
        ctx.send(&PongResponse {
            payload: packet.payload,
        })
        .and_then(|()| ctx.conn.close()),
    )
}

/// Login: the one step that genuinely has to wait for something external.
///
/// The handler returns [`Flow::later`], so the driver stops reading from this connection until
/// authentication resolves. That is what keeps the flow readable *and* race-free: no second packet
/// can be dispatched into a half-authenticated session.
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Outcome<Session> {
    let conn = ctx.conn.clone();
    let version = ctx.version();
    let claimed = packet.user_name.clone();

    Flow::later(async move {
        let (name, id) = authenticate(&claimed).await?;

        conn.send(&LoginSuccess {
            user_id: id,
            user_name: name.clone(),
            properties: vec![Property {
                name: "textures".to_owned(),
                value: "<signed>".to_owned(),
            }],
            // The gated field is filled from the feature, not from a version number. On older
            // clients this stays `None` and never reaches the wire.
            session_id: version
                .has(Feature::LoginSuccessSessionId)
                .then(Uuid::new_v4),
        })?;

        Ok(Update::apply(move |session: &mut Session| {
            session.claimed_name = claimed;
            session.profile = Some((name, id));
        }))
    })
}

/// The client acknowledged the login: enter configuration and start looking for a backend.
///
/// Backend selection is [`detached`](crate::conn::ConnHandle::detach) because it has to overlap with
/// keep-alives -- the client will disconnect itself if we go quiet for 20 seconds. The detached task
/// sends the transfer itself and reports what it did back into state.
fn on_login_acknowledged(ctx: Ctx<'_, Session>, _packet: LoginAcknowledged) -> Outcome<Session> {
    ctx.conn.set_phase(Phase::Configuration);

    let conn = ctx.conn.clone();
    Flow::from_result(ctx.conn.detach(async move {
        let target = select_backend().await?;
        conn.send(&Transfer {
            host: target.0,
            port: VarInt(target.1),
        })?;
        conn.close()?;
        Ok(Update::none())
    }))
}

/// A keep-alive came back: clear the outstanding id.
fn on_keep_alive_response(ctx: Ctx<'_, Session>, packet: KeepAliveResponse) -> Outcome<Session> {
    if ctx.state.awaiting_keep_alive == Some(packet.id) {
        ctx.state.awaiting_keep_alive = None;
    }
    Flow::done()
}

/// The tick: send a keep-alive, and fail the connection if the previous one went unanswered.
///
/// The driver owns the timer, so this is the only place keep-alive policy lives -- rather than a
/// `select!` arm tangled into the middle of a 470-line function.
fn on_tick(ctx: Ctx<'_, Session>) -> Outcome<Session> {
    if ctx.phase() != Phase::Configuration {
        return Flow::done();
    }

    if ctx.state.awaiting_keep_alive.is_some() {
        ctx.state.keep_alive_misses += 1;
        return Flow::fail(Error::Internal(InternalError::Handler(
            "client missed a keep-alive".into(),
        )));
    }

    let id = ctx.state.next_keep_alive;
    ctx.state.next_keep_alive += 1;
    ctx.state.awaiting_keep_alive = Some(id);
    Flow::from_result(ctx.send(&KeepAlive { id }))
}

/// Stands in for the authentication adapter.
async fn authenticate(claimed_name: &str) -> Result<(String, Uuid)> {
    tokio::time::sleep(Duration::from_millis(1)).await;
    Ok((claimed_name.to_owned(), Uuid::new_v4()))
}

/// Stands in for the discovery adapter chain.
async fn select_backend() -> Result<(String, i32)> {
    tokio::time::sleep(Duration::from_millis(1)).await;
    Ok(("backend-1.justchunks.net".to_owned(), 25565))
}
