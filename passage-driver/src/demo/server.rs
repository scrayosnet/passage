//! A worked server on top of the driver: the Passage flow, as handlers.
//!
//! This is the layer the README calls "a basic server implementation on top of the backbone". It
//! owns no I/O and no framing -- only the state machine. Compare it to the 470-line `listen()` it
//! replaces: each step is a function you can read, test and override in isolation, and the
//! sequential parts stay sequential because the connection applies one handler's operations before it
//! looks at the next packet.
//!
//! Note what no handler here does: hold a lock, mutate state in place, or await. Every one of them
//! reads [`Ctx::state`] and queues what it wants to happen.

use crate::conn::{Ctx, Ending};
use crate::demo::packets::{
    Intent, Intention, KeepAlive, KeepAliveResponse, LoginAcknowledged, LoginDisconnect,
    LoginStart, LoginSuccess, PingRequest, PongResponse, Property, StatusRequest, StatusResponse,
    Transfer,
};
use crate::error::{BuildError, Class, Error, ProtocolError, Result};
use crate::packet::Phase;
use crate::router::{Router, UnknownPolicy};
use crate::server::Finished;
use crate::version::{ProtocolVersion, versions};
use std::net::SocketAddr;
use std::time::Duration;
use tracing::{debug, warn};
use uuid::Uuid;

/// The oldest protocol version that can log in: 1.20.5 introduced the configuration phase, cookies
/// and the transfer packet, all of which Passage depends on.
pub const MIN_LOGIN_VERSION: ProtocolVersion = versions::V1_20_5;

/// Per-connection state.
///
/// Plain data: no locks, no atomics, no `Arc`. Handlers read it through [`Ctx::state`] and change
/// it by queueing [`Ctx::update`].
#[derive(Debug, Default)]
pub struct Session {
    /// Where the connection came from, as reported by the listener.
    ///
    /// [`serve`](crate::server::serve) calls the state factory once per accepted socket, which is
    /// how a per-connection fact like this gets into the state before the first packet arrives.
    pub peer: Option<SocketAddr>,
    /// The hostname the client connected to.
    pub host: String,
    /// What the client asked for.
    pub intent: Intent,
    /// The name the client claimed in [`LoginStart`].
    pub claimed_name: String,
    /// The profile as verified by the authentication adapter.
    pub profile: Option<(String, Uuid)>,
    /// Why the peer was turned away, if it was.
    ///
    /// This is where "we refused them" lives, rather than in a variant of the driver's own
    /// vocabulary. The driver knows the connection closed; only this handler knows it was a refusal
    /// *and* what for -- and a low-cardinality reason like this is what a metric wants anyway.
    /// [`Outcome::state`](crate::conn::Outcome::state) hands it back to whoever reports.
    pub refused: Option<&'static str>,
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
/// It names no protocol versions. The packets carry their own ([`Packet::IDS`](crate::packet::Packet::IDS)),
/// so building reads the thresholds out of them and produces one dispatch table per version at
/// which dispatch actually changes -- which is why a client on a release nobody thought to write
/// down is served exactly like its neighbours.
pub fn router() -> std::result::Result<Router<Session>, BuildError> {
    Router::builder()
        .unknown(UnknownPolicy::Reject)
        .on::<Intention>(on_intention)
        .on::<StatusRequest>(on_status_request)
        .on::<PingRequest>(on_ping_request)
        .on::<LoginStart>(on_login_start)
        .on::<LoginAcknowledged>(on_login_acknowledged)
        .on::<KeepAliveResponse>(on_keep_alive_response)
        .on_tick(on_tick)
        .on_error(on_error)
        .build()
}

/// The handshake: pick a phase, pin the version, reject what we cannot serve.
fn on_intention(ctx: Ctx<'_, Session>, packet: Intention) -> Result<()> {
    let version = packet.protocol_version;
    let phase = match packet.intent {
        Intent::Status => Phase::Status,
        Intent::Login | Intent::Transfer => Phase::Login,
    };

    // Both of these are queued before the refusal below, and that ordering is the whole point:
    // operations drain before `on_error` is asked for a disconnect message, so it finds the
    // connection already in the phase whose disconnect packet it knows how to send.
    ctx.set_version(version)?;
    ctx.set_phase(phase)?;

    // Status has to work for every version -- that is how old clients get told which version to
    // use. Logging in does not, and neither does a version the table cannot place: a snapshot is
    // numerically above every release, so every gated field would be written for a client that
    // cannot read it.
    if phase != Phase::Status && !(version.is_release() && version.at_least(MIN_LOGIN_VERSION)) {
        return Err(ProtocolError::UnsupportedVersion { version }.into());
    }

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
    // Sequence within a phase is the handler's business, not the connection's: nothing stops a
    // client sending this twice, so the check is here, against the state that records it.
    if ctx.state.profile.is_some() {
        return Err(Error::peer("duplicate_login", "the client logged in twice"));
    }

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
    if ctx.state.profile.is_none() {
        return Err(Error::peer(
            "premature_acknowledgement",
            "the client acknowledged a login it never started",
        ));
    }

    ctx.set_phase(Phase::Configuration)?;

    let conn = ctx.conn.clone();
    ctx.spawn(async move {
        let (host, port) = select_backend().await?;
        // One batch, because a handle can be held anywhere: this one is on the connection's own
        // task, but the copy the connection builder hands back is not, and anything queued from there
        // could otherwise land between the transfer and the close.
        conn.batch(|batch| {
            batch.send(Transfer { host, port })?;
            batch.close();
            Ok(())
        })
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
/// The connection owns the timer, so this is the only place keep-alive policy lives -- rather than a
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

/// The last word: tell the client why, when there is a way to say it.
///
/// Called for everything nobody asked for -- a refused login, a missed keep-alive, an expired
/// deadline, a shutdown. Without it all of those look identical from the client's side, which is
/// "Internal Exception" and a bug report.
///
/// It only speaks in the login phase, because the configuration-phase disconnect carries a
/// network-NBT text component and [`wire`](crate::wire) has no NBT yet -- a gap in the demo's
/// packet set, not in the mechanism. Anywhere else it says nothing, which is what an ordinary
/// `Ok(())` means here.
///
/// Failing is safe: an error from this handler is logged and the connection still reports the
/// ending it already had. So the `?` below is not a risk -- if the packet cannot be encoded for a
/// client whose version was never pinned, the batch queues nothing and the refusal is still the
/// refusal.
fn on_error(ctx: Ctx<'_, Session>, ending: &Ending) -> Result<()> {
    // Runs for *every* ending we did not ask for, a peer that simply vanished included. That is the
    // half of this that has nothing to do with messages: whatever the connection reserved on the way
    // in has to be released on the way out, and a client disappearing mid-login needs that exactly
    // as much as one that timed out. In Passage this is where the discovery adapter's reservation
    // goes back; the demo has nothing to release, so it settles for noticing.
    if let Some((name, _)) = &ctx.state.profile {
        debug!(
            player = name,
            reason = ending.label(),
            "authenticated player left before transfer"
        );
    }

    // Everything below writes, so everything below needs somebody to write to and a phase with a
    // packet to write it in.
    if !ending.can_reply() || ctx.phase() != Phase::Login {
        return Ok(());
    }

    // Two forms of the same fact: one for the player, one for us. Real Passage runs the first
    // through the localization adapter with the locale the client sent in the configuration phase;
    // a demo has one language.
    let (label, reason) = match ending {
        Ending::Cancelled => ("shutdown", "The server is restarting. Please reconnect."),
        Ending::TimedOut => ("timeout", "Took too long to get you a server."),
        Ending::Failed(err) => match err.class() {
            Class::Peer => ("bad_client", "Your client sent something we could not use."),
            Class::Transport => ("transport", "The connection broke."),
            Class::Internal => ("internal", "Something went wrong on our side."),
        },
        // Refused by the guard above.
        Ending::PeerClosed => return Ok(()),
    };

    ctx.batch(|batch| {
        batch.send(LoginDisconnect::text(reason))?;
        // Recorded in the state, so the connection can end as an ordinary `Closed` and the report
        // still knows this one was a refusal -- with the reason, which no completion could carry.
        batch.update(move |session: &mut Session| session.refused = Some(label));
        batch.close();
        Ok(())
    })
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
/// caller's policy. What the driver provides is enough information to decide -- the peer, the
/// duration, the state the connection was left in, and an [`Ending`] it did not choose with the
/// [`Class`] of anything that went wrong. Everything the old implementation's metrics needed is in
/// here, which was the point.
///
/// This is also the piece the previous implementation could not express: `Err(ConnectionClosed)`
/// meant both "done" and "broken", so every call site had to special-case it and any new error
/// variant silently fell into the wrong bucket.
pub fn log_completion(finished: &Finished<'_, Session, SocketAddr>) {
    let peer = finished.addr;
    let host = finished.state().map_or("", |session| session.host.as_str());
    let elapsed = finished.elapsed;

    // A refusal ends as an ordinary completion, so this is what tells the two apart -- and it comes
    // out of the session, because the handler that refused is the one that knew why.
    if let Some(refused) = finished.state().and_then(|session| session.refused) {
        debug!(?peer, host, ?elapsed, reason = refused, "peer refused");
        return;
    }

    // One match over every way a connection can end, because there is one type for it.
    match finished.result {
        Ok(()) => debug!(?peer, host, ?elapsed, "connection finished"),
        // Not a failure, and not something to log loudly: a scanner that took its MOTD and left
        // looks exactly like this.
        Err(Ending::PeerClosed) => debug!(?peer, host, ?elapsed, "peer hung up"),
        Err(Ending::Cancelled) => {
            debug!(?peer, host, ?elapsed, "connection cancelled by shutdown");
        }
        Err(Ending::TimedOut) => debug!(?peer, host, ?elapsed, "connection timed out"),
        Err(Ending::Failed(err)) => match err.class() {
            Class::Peer | Class::Transport => {
                debug!(?peer, host, ?elapsed, cause = %err, kind = err.label(), "connection dropped");
            }
            Class::Internal => {
                warn!(?peer, host, ?elapsed, cause = %err, kind = err.label(), "connection failed");
            }
        },
    }
}
