//! A worked server on top of the driver: the Passage flow, as handlers.
//!
//! This is the layer the README calls "a basic server implementation on top of the backbone". It
//! owns no I/O and no framing -- only the state machine. Compare it to the 470-line `listen()` it
//! replaces: each step is a function you can read, test and override in isolation, and the
//! sequential parts stay sequential because the driver applies one handler's operations before it
//! looks at the next packet.
//!
//! Note what no handler here does: hold a lock, mutate state in place, or await. Every one of them
//! reads [`Ctx::state`] and queues what it wants to happen.

use crate::conn::Ctx;
use crate::demo::packets::{
    Intent, Intention, KeepAlive, KeepAliveResponse, LoginAcknowledged, LoginStart, LoginSuccess,
    PingRequest, PongResponse, Property, StatusRequest, StatusResponse, Transfer,
};
use crate::driver::Completion;
use crate::error::{BuildError, Class, Error, ProtocolError, Result};
use crate::packet::{Direction, Phase};
use crate::router::{Router, UnknownPolicy};
use crate::version::{ProtocolVersion, versions};
use std::time::Duration;
use tracing::{debug, warn};
use uuid::Uuid;

/// The oldest protocol version that can log in: 1.20.5 introduced the configuration phase, cookies
/// and the transfer packet, all of which Passage depends on.
pub const MIN_LOGIN_VERSION: ProtocolVersion = versions::V1_20_5;

/// The versions this server builds dispatch tables for.
///
/// Anything outside this list still gets a status ping answered, from the version-independent
/// table, which is how an old client learns what to install.
pub const SUPPORTED_VERSIONS: &[ProtocolVersion] =
    &[versions::V1_20_5, versions::V1_21, versions::V26_2];

/// Per-connection state.
///
/// Plain data: no locks, no atomics, no `Arc`. Handlers read it through [`Ctx::state`] and change
/// it by queueing [`Ctx::update`].
#[derive(Debug, Default)]
pub struct Session {
    /// The hostname the client connected to.
    pub host: String,
    /// What the client asked for.
    pub intent: Intent,
    /// The name the client claimed in [`LoginStart`].
    pub claimed_name: String,
    /// The profile as verified by the authentication adapter.
    pub profile: Option<(String, Uuid)>,
    /// The keep-alive we are waiting for an answer to.
    pub awaiting_keep_alive: Option<i64>,
    /// The next keep-alive ID, so the demo stays deterministic.
    pub next_keep_alive: i64,
}

/// Builds the server router.
///
/// This is the whole protocol surface of the server, in one screen: which packets it accepts, in
/// which phase, and what runs for each. Adding a packet is one line here plus one `impl Packet` --
/// no trait to widen, no `match` arm to forget.
///
/// Building resolves every ID at every supported version, so a collision introduced by a new packet
/// fails here rather than on the first client that sends it.
pub fn router() -> std::result::Result<Router<Session>, BuildError> {
    Router::builder(Direction::Serverbound)
        .unknown(UnknownPolicy::Reject)
        .on::<Intention, _>(on_intention)
        .on::<StatusRequest, _>(on_status_request)
        .on::<PingRequest, _>(on_ping_request)
        .on::<LoginStart, _>(on_login_start)
        .on::<LoginAcknowledged, _>(on_login_acknowledged)
        .on::<KeepAliveResponse, _>(on_keep_alive_response)
        .on_tick(on_tick)
        .build(SUPPORTED_VERSIONS.iter().copied())
}

/// The handshake: pick a phase, pin the version, reject what we cannot serve.
fn on_intention(ctx: Ctx<'_, Session>, packet: Intention) -> Result<()> {
    let version = packet.protocol_version;

    // A negative version is not a version. Refuse it here rather than letting every later ID lookup
    // silently miss.
    if version.get() < 0 {
        return Err(ProtocolError::UnsupportedVersion { version }.into());
    }

    // Status has to work for every version -- that is how old clients get told which version to
    // use. Logging in does not.
    if packet.intent != Intent::Status && !version.at_least(MIN_LOGIN_VERSION) {
        return Err(ProtocolError::UnsupportedVersion { version }.into());
    }

    ctx.set_version(version)?;
    ctx.set_phase(match packet.intent {
        Intent::Status => Phase::Status,
        Intent::Login | Intent::Transfer => Phase::Login,
    })?;

    let Intention {
        server_address,
        intent,
        ..
    } = packet;
    ctx.update(move |session: &mut Session| {
        session.host = server_address;
        session.intent = intent;
    })
}

/// The status response. Nothing external to wait for, so nothing to spawn.
fn on_status_request(ctx: Ctx<'_, Session>, _packet: StatusRequest) -> Result<()> {
    let body = format!(
        r#"{{"version":{{"name":"1.21","protocol":{}}},"players":{{"max":0,"online":0}},"description":{{"text":"{}"}}}}"#,
        ctx.version().get(),
        ctx.state.host,
    );
    ctx.send(StatusResponse { body })
}

/// The latency probe, and the end of a status connection.
fn on_ping_request(ctx: Ctx<'_, Session>, packet: PingRequest) -> Result<()> {
    ctx.send(PongResponse {
        payload: packet.payload,
    })?;
    // Queued behind the pong, so the client gets its answer before the socket goes away.
    ctx.close()
}

/// Login: the one step that genuinely has to wait for something external.
///
/// [`Ctx::exclusive`] says the peer is expected to stay quiet until authentication resolves. That
/// keeps the flow readable *and* race-free: no second packet can be dispatched into a
/// half-authenticated session, and one that arrives anyway is reported instead of replayed.
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Result<()> {
    let conn = ctx.conn.clone();
    // The version never changes after the handshake, so capturing it is safe.
    let version = ctx.version();
    let claimed = packet.user_name;

    ctx.exclusive(async move {
        let (name, id) = authenticate(&claimed).await?;

        // Both are operations, so they land in exactly this order: the profile is recorded before
        // the packet that announces it reaches the wire. A tick in between cannot observe a session
        // whose login has been announced but not stored.
        let profile = (name.clone(), id);
        conn.update(move |session: &mut Session| {
            session.claimed_name = claimed;
            session.profile = Some(profile);
        })?;

        conn.send(LoginSuccess {
            user_id: id,
            user_name: name,
            properties: vec![Property {
                name: "textures".to_owned(),
                value: "<signed>".to_owned(),
            }],
            // On older clients this stays `None` and never reaches the wire. If this and the
            // codec ever disagreed about the threshold, the encoder would refuse the packet rather
            // than truncate it -- so the duplication fails closed.
            session_id: version.at_least(versions::V26_2).then(Uuid::new_v4),
        })
    })
}

/// The client acknowledged the login: enter configuration and start looking for a backend.
///
/// Backend selection uses [`Ctx::spawn`] rather than [`Ctx::exclusive`] because it has to overlap
/// with keep-alives -- the client disconnects itself if we go quiet for 20 seconds. So the gate
/// stays open, the tick handler keeps running, and the task sends the transfer when it is done.
fn on_login_acknowledged(ctx: Ctx<'_, Session>, _packet: LoginAcknowledged) -> Result<()> {
    ctx.set_phase(Phase::Configuration)?;

    let conn = ctx.conn.clone();
    ctx.spawn(async move {
        let (host, port) = select_backend().await?;
        conn.send(Transfer { host, port })?;
        conn.close()
    })
}

/// A keep-alive came back: clear the outstanding ID.
///
/// The comparison happens inside the update, where the value it is compared against is the current
/// one, rather than one read a moment earlier.
fn on_keep_alive_response(ctx: Ctx<'_, Session>, packet: KeepAliveResponse) -> Result<()> {
    ctx.update(move |session: &mut Session| {
        if session.awaiting_keep_alive == Some(packet.id) {
            session.awaiting_keep_alive = None;
        }
    })
}

/// The tick: send a keep-alive, and fail the connection if the previous one went unanswered.
///
/// The driver owns the timer, so this is the only place keep-alive policy lives -- rather than a
/// `select!` arm tangled into the middle of a 470-line function.
fn on_tick(ctx: Ctx<'_, Session>) -> Result<()> {
    if ctx.phase() != Phase::Configuration {
        return Ok(());
    }

    if ctx.state.awaiting_keep_alive.is_some() {
        // A client that stops answering is the *peer's* problem, and is classified as one. As an
        // internal error it would be logged at `warn` and reported to error tracking, which is the
        // wrong treatment for ordinary client behaviour.
        return Err(Error::peer(
            "keep_alive_timeout",
            "the client missed a keep-alive",
        ));
    }

    let id = ctx.state.next_keep_alive;
    ctx.update(move |session: &mut Session| {
        session.next_keep_alive += 1;
        session.awaiting_keep_alive = Some(id);
    })?;
    ctx.send(KeepAlive { id })
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

/// Logs a finished connection at the level its outcome deserves.
///
/// This lives with the server rather than in the driver: what to log, and how loudly, is the
/// caller's policy. What the driver provides is enough information to decide --
/// [`Completion`] for the happy paths and [`Class`] for who is to blame otherwise.
///
/// This is the piece the previous implementation could not express: `Err(ConnectionClosed)` meant
/// both "done" and "broken", so every call site had to special-case it and any new error variant
/// silently fell into the wrong bucket.
pub fn log_completion(result: &Result<Completion>) {
    match result {
        Ok(completion) => debug!(?completion, "connection finished"),
        Err(err) => match err.class() {
            Class::Peer | Class::Transport => {
                debug!(cause = %err, kind = err.label(), "connection dropped");
            }
            Class::Internal => {
                warn!(cause = %err, kind = err.label(), "connection failed");
            }
        },
    }
}
