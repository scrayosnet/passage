//! End-to-end tests for the handlers, driven the way a Minecraft client drives them.
//!
//! The test is the client: it puts real frames on a socket, switches to the cipher when the
//! protocol says to, and reads what comes back. Nothing here reaches into the handlers, so what is
//! proven is the conversation rather than the code that produces it.

use crate::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::held::{HeldAuthenticationAdapter, HeldStatusAdapter};
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::cookie::{AuthCookie, Cookie, SessionCookie};
use crate::crypto;
use crate::router::DynRoute;
use crate::router::state::State;
use futures::{SinkExt, StreamExt};
use passage_adapters::authentication::Profile;
use passage_adapters::{
    DisabledAuthenticationAdapter, FixedDiscoveryAdapter, FixedLocalizationAdapter,
    FixedStatusAdapter, Target,
};
use passage_core::client::{Client, Connected};
use passage_core::codec::{Aes128Cfb8, Frame, FrameCodec};
use passage_core::common::{ChatMode, DisplayedSkinParts, MainHand, State as Intent};
use passage_core::connection::Options;
use passage_core::packet::{configuration, handshake, login, status};
use passage_core::wire::{Bytes, BytesMut, Options as WireOptions, Reader, Writer};
use passage_core::{Packet, ProtocolVersion, versions};
use regex::Regex;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;
use uuid::Uuid;

/// The address the test client connects from.
fn peer() -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 25_565))
}

/// The host the route below matches, and what the client puts in its handshake.
const HOST: &str = "mc.justchunks.net";

/// Every protocol version Passage serves, breakpoints and the releases between them alike.
///
/// The conversations below are replayed once per entry. The in-between versions are not padding:
/// a packet whose ID table names the wrong threshold is correct at the breakpoints on either side
/// and wrong only in the middle, which is exactly the shape of bug that reaches production.
const SUPPORTED: std::ops::RangeInclusive<i32> = 766..=777;

/// One route, answered entirely from memory: no session server, no discovery service.
fn routes() -> Arc<[Arc<DynRoute>]> {
    routes_with(
        DynStatusAdapter::Fixed(FixedStatusAdapter::new(
            None,
            versions::V26_3,
            versions::V1_20_5,
            versions::V26_3,
        )),
        DynAuthenticationAdapter::Disabled(DisabledAuthenticationAdapter::new()),
    )
}

/// The same route, with the two adapters a test may want to hold open.
fn routes_with(
    status_adapter: DynStatusAdapter,
    authentication_adapter: DynAuthenticationAdapter,
) -> Arc<[Arc<DynRoute>]> {
    let target = Target {
        identifier: "backend-1".to_owned(),
        address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 25_566)),
        priority: 0,
        meta: Default::default(),
    };
    let route = Route {
        hostname: Regex::new(HOST).expect("a pattern"),
        status: status_adapter,
        discovery: vec![DynDiscoveryActionAdapter::FixedDiscovery(
            FixedDiscoveryAdapter::new(vec![target]),
        )],
        authentication: authentication_adapter,
        localization: DynLocalizationAdapter::Fixed(FixedLocalizationAdapter::new(
            "en_GB".to_owned(),
            Default::default(),
            true,
        )),
    };
    Arc::from(vec![Arc::new(route)])
}

/// The secret the auth cookie is signed with, for the transfer that presents one.
const SECRET: &[u8] = b"a-shared-secret";

/// Runs one connection against the router, and hands back the socket the client holds and what the
/// connection ended for.
fn serve() -> (TestClient, JoinHandle<Option<&'static str>>) {
    serve_with(None)
}

/// The same, for a router that signs auth cookies with `secret`.
fn serve_with(secret: Option<&'static [u8]>) -> (TestClient, JoinHandle<Option<&'static str>>) {
    serve_routes(routes(), secret)
}

/// The same, over a route whose adapters the test built.
fn serve_routes(
    routes: Arc<[Arc<DynRoute>]>,
    secret: Option<&'static [u8]>,
) -> (TestClient, JoinHandle<Option<&'static str>>) {
    let (server_io, client_io) = tokio::io::duplex(8192);
    let secret = secret.map(Bytes::from_static);
    let router = Arc::new(crate::router::router().expect("the router builds"));

    let state_routes = Arc::clone(&routes);
    let connection = Client::new(Connected::new(server_io, peer()))
        .state(move |addr: &SocketAddr| {
            State::new(Arc::clone(&state_routes), *addr, secret.clone(), 3_600)
        })
        .dispatch(router)
        .config(Options {
            initial_version: ProtocolVersion::UNKNOWN,
            ..Options::default()
        });

    let served = tokio::spawn(async move {
        let outcome = connection.connect().await.expect("preconnected");
        outcome.error.map(|error| {
            assert!(
                error.is_peer_error(),
                "the connection failed on our side: {error}",
            );
            error.reason()
        })
    });
    (TestClient::new(client_io), served)
}

/// The client half of the conversation: frames in, frames out, and the cipher in between.
struct TestClient {
    framed: Framed<DuplexStream, FrameCodec>,
    version: ProtocolVersion,
    options: WireOptions,
}

impl TestClient {
    fn new(io: DuplexStream) -> Self {
        let options = WireOptions::default();
        Self {
            framed: Framed::new(io, FrameCodec::new(options)),
            version: versions::V26_3,
            options,
        }
    }

    /// Sends a packet, encoded for the version this client speaks.
    async fn send<P: Packet>(&mut self, packet: P) {
        let frame = Frame::of(&packet, self.version, self.options).expect("encodes");
        self.framed.send(frame).await.expect("writes");
    }

    /// Sends a packet under an ID of the test's choosing, for the versions where the packet has
    /// none of its own: a client below 1.20.5 still sends a login start, and Passage still has to
    /// do something sensible with it.
    async fn send_as<P: Packet>(&mut self, id: i32, packet: P) {
        let mut buf = BytesMut::new();
        {
            let mut writer = Writer::new(&mut buf).with_options(self.options);
            writer.var_int(id);
            packet.encode(&mut writer, self.version).expect("encodes");
        }
        let frame = Frame {
            name: P::NAME,
            id,
            payload: buf.freeze(),
        };
        self.framed.send(frame).await.expect("writes");
    }

    /// Reads the next frame without asking which packet it is, for the versions whose IDs this
    /// crate does not claim to know.
    async fn next_raw(&mut self) -> Option<Frame> {
        self.framed
            .next()
            .await
            .map(|frame| frame.expect("the frame decodes"))
    }

    /// Reads the next packet, which must be the one asked for.
    async fn expect<P: Packet>(&mut self) -> P {
        let frame = self
            .framed
            .next()
            .await
            .expect("the connection stays open")
            .expect("the frame decodes");
        assert_eq!(
            Some(frame.id),
            P::id(self.version),
            "expected {}, got id {:#04x}",
            P::NAME,
            frame.id,
        );
        let mut reader = Reader::new(frame.payload).with_options(self.options);
        reader.var_int("packet_id").expect("the ID leads");
        P::decode(&mut reader, self.version).expect("the packet decodes")
    }

    /// Encrypts everything from here on, the way the client does once it has sent the secret.
    fn encrypt(&mut self, secret: &[u8]) {
        self.framed
            .codec_mut()
            .set_cipher(Box::new(Aes128Cfb8::new(secret).expect("a key")));
    }

    /// Asserts the server hung up.
    async fn expect_eof(&mut self) {
        assert!(
            self.framed.next().await.is_none(),
            "expected the server to close the connection",
        );
    }
}

#[tokio::test]
async fn a_status_ping_is_answered_at_every_supported_version() {
    for protocol in SUPPORTED {
        a_status_ping_is_answered_and_the_connection_closed(ProtocolVersion::new(protocol)).await;
    }
}

async fn a_status_ping_is_answered_and_the_connection_closed(version: ProtocolVersion) {
    let (mut client, served) = serve();
    client.version = version;

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: version,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Status,
        })
        .await;
    client.send(status::ClientStatusRequestPacket).await;

    let status = client.expect::<status::ServerStatusResponsePacket>().await;
    assert!(!status.body.is_empty(), "the status is answered");

    client
        .send(status::ClientPingRequestPacket { payload: 0x1234 })
        .await;
    let pong = client.expect::<status::ServerPongResponsePacket>().await;
    assert_eq!(pong.payload, 0x1234);

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), None, "a clean ending");
}

#[tokio::test]
async fn a_hostname_no_route_matches_is_refused() {
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: "elsewhere.example".into(),
            server_port: 25_565,
            next_state: Intent::Status,
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("no_route"));
}

#[tokio::test]
async fn a_login_runs_to_the_transfer_packet_at_every_supported_version() {
    // The whole conversation, replayed for every version Passage claims to serve. This is what
    // would have caught 26.3 moving the transfer packet and 26.2 adding the session ID: both are
    // invisible at the version the suite used to pin, and fatal one version over.
    for protocol in SUPPORTED {
        a_login_runs_to_the_transfer_packet(ProtocolVersion::new(protocol)).await;
    }
}

async fn a_login_runs_to_the_transfer_packet(version: ProtocolVersion) {
    let (mut client, served) = serve();
    client.version = version;

    // Handshake and login start, in the clear.
    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: version,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            name: "Hydrofin".into(),
            uuid: Uuid::from_u128(7),
        })
        .await;

    // The encryption request carries the token the client has to hand back, and the public key it
    // is encrypted with.
    let encryption = client
        .expect::<login::ServerEncryptionRequestPacket>()
        .await;
    assert!(
        !encryption.verify_token.is_empty(),
        "the client has nothing to answer with otherwise",
    );
    assert!(
        !encryption.should_authenticate,
        "the route authenticates nobody, so the client must not be sent to Mojang",
    );

    let public = crypto::KEY_PAIR.1.clone();
    let secret = b"0123456789abcdef";
    client
        .send(login::ClientEncryptionResponsePacket {
            shared_secret: crypto::encrypt(&public, secret).expect("encrypts").into(),
            verify_token: crypto::encrypt(&public, &encryption.verify_token)
                .expect("encrypts")
                .into(),
        })
        .await;

    // Everything from here is AES-128-CFB8, in both directions. The cipher was queued before the
    // packet below, so the server's half switched at the same point in the stream.
    client.encrypt(secret);

    // The session cookie is asked for once the connection is private. This client has none.
    let request = client.expect::<login::ServerCookieRequestPacket>().await;
    assert_eq!(request.key, SessionCookie::KEY);
    client
        .send(login::ClientCookieResponsePacket {
            key: request.key,
            payload: None,
        })
        .await;

    let success = client.expect::<login::ServerLoginSuccessPacket>().await;
    assert_eq!(success.name, "Hydrofin");
    if version.at_least(versions::V26_2) {
        // The session is minted before the login success rather than at the transfer, so even a
        // client that arrived without one is told which session it is in.
        assert!(
            success.session_id.is_some_and(|id| !id.is_nil()),
            "a client without a session is given one before it is named a session",
        );
    }
    client.send(login::ClientLoginAcknowledgedPacket).await;

    // The configuration phase: the client says who it is, and the server answers with a target.
    client
        .send(configuration::ClientClientInformationPacket {
            locale: "en_GB".into(),
            view_distance: 12,
            chat_mode: ChatMode::Enabled,
            chat_colors: true,
            displayed_skin_parts: DisplayedSkinParts(0x7f),
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_status: None,
        })
        .await;

    // A client that had no session is given one before it is sent on.
    let store = client
        .expect::<configuration::ServerStoreCookiePacket>()
        .await;
    assert_eq!(store.key, SessionCookie::KEY);
    let session = SessionCookie::decode(None, &store.payload)
        .expect("decodes")
        .expect("a session");
    assert_eq!(session.server_address, HOST);

    let transfer = client.expect::<configuration::ServerTransferPacket>().await;
    assert_eq!(transfer.host, "10.0.0.1");
    assert_eq!(transfer.port, 25_566);

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), None, "a clean ending");
}

#[tokio::test]
async fn a_client_that_brings_a_session_keeps_it_and_is_handed_it_back() {
    // The session is the thread between two hops, so the one the client arrives with is the one it
    // leaves with: a second ID minted here would make the same player look like two, and the
    // address in it has to stay the one the player first connected to rather than being rewritten
    // to whatever hostname this hop was reached under.
    //
    // It is handed back on every hop and not only to a client that had none, because the trace
    // inside it is refreshed to this connection's -- that is what makes the next hop link to the
    // one it came from instead of to the first in the chain.
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            name: "Hydrofin".into(),
            uuid: Uuid::from_u128(7),
        })
        .await;

    let encryption = client
        .expect::<login::ServerEncryptionRequestPacket>()
        .await;
    let public = crypto::KEY_PAIR.1.clone();
    let secret = b"0123456789abcdef";
    client
        .send(login::ClientEncryptionResponsePacket {
            shared_secret: crypto::encrypt(&public, secret).expect("encrypts").into(),
            verify_token: crypto::encrypt(&public, &encryption.verify_token)
                .expect("encrypts")
                .into(),
        })
        .await;
    client.encrypt(secret);

    // The session this client was given on an earlier hop, under a hostname that is not this one.
    let presented = SessionCookie {
        id: Uuid::from_u128(11),
        server_address: "first.justchunks.net".to_owned(),
        server_port: 25_564,
        extra: Default::default(),
    };
    let request = client.expect::<login::ServerCookieRequestPacket>().await;
    assert_eq!(request.key, SessionCookie::KEY);
    client
        .send(login::ClientCookieResponsePacket {
            key: request.key,
            payload: Some(presented.encode(None).expect("encodes")),
        })
        .await;

    let success = client.expect::<login::ServerLoginSuccessPacket>().await;
    assert_eq!(
        success.session_id,
        Some(presented.id),
        "the session the client brought, not a fresh one",
    );
    client.send(login::ClientLoginAcknowledgedPacket).await;
    client
        .send(configuration::ClientClientInformationPacket {
            locale: "en_GB".into(),
            view_distance: 12,
            chat_mode: ChatMode::Enabled,
            chat_colors: true,
            displayed_skin_parts: DisplayedSkinParts(0x7f),
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
            particle_status: None,
        })
        .await;

    let store = client
        .expect::<configuration::ServerStoreCookiePacket>()
        .await;
    assert_eq!(store.key, SessionCookie::KEY);
    let returned = SessionCookie::decode(None, &store.payload)
        .expect("decodes")
        .expect("a session");
    assert_eq!(returned.id, presented.id, "the same session, not a new one");
    assert_eq!(
        returned.server_address, presented.server_address,
        "where the player first came in, not where this hop was reached",
    );
    assert_eq!(returned.server_port, presented.server_port);

    let transfer = client.expect::<configuration::ServerTransferPacket>().await;
    assert_eq!(transfer.host, "10.0.0.1");

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), None, "a clean ending");
}

#[tokio::test]
async fn a_transfer_with_a_valid_auth_cookie_skips_authentication() {
    let (mut client, served) = serve_with(Some(SECRET));

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Transfer,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            // The name in the login start is not what a transferred player is known by: the cookie
            // is, because that one is signed.
            name: "Impostor".into(),
            uuid: Uuid::from_u128(1),
        })
        .await;

    // A transfer is asked for the auth cookie, in the clear, before anything is encrypted.
    let request = client.expect::<login::ServerCookieRequestPacket>().await;
    assert_eq!(request.key, AuthCookie::KEY);

    let cookie = AuthCookie {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time runs forwards")
            .as_secs(),
        client_addr: peer(),
        user_name: "Hydrofin".into(),
        user_id: Uuid::from_u128(7),
        target: None,
        profile_properties: Vec::new(),
        extra: Default::default(),
    };
    client
        .send(login::ClientCookieResponsePacket {
            key: request.key,
            payload: Some(cookie.encode(Some(SECRET)).expect("encodes")),
        })
        .await;

    // Having accepted the cookie, the server does not ask the authentication adapter.
    let encryption = client
        .expect::<login::ServerEncryptionRequestPacket>()
        .await;
    assert!(
        !encryption.should_authenticate,
        "the cookie is what vouches for this player",
    );

    let public = crypto::KEY_PAIR.1.clone();
    let secret = b"0123456789abcdef";
    client
        .send(login::ClientEncryptionResponsePacket {
            shared_secret: crypto::encrypt(&public, secret).expect("encrypts").into(),
            verify_token: crypto::encrypt(&public, &encryption.verify_token)
                .expect("encrypts")
                .into(),
        })
        .await;
    client.encrypt(secret);

    let request = client.expect::<login::ServerCookieRequestPacket>().await;
    assert_eq!(request.key, SessionCookie::KEY);
    client
        .send(login::ClientCookieResponsePacket {
            key: request.key,
            payload: None,
        })
        .await;

    let success = client.expect::<login::ServerLoginSuccessPacket>().await;
    assert_eq!(
        success.name, "Hydrofin",
        "the signed name, not the sent one"
    );
    assert_eq!(success.uuid, Uuid::from_u128(7));
    served.abort();
}

/// Opens a login and stops at the point the server asks for the auth cookie.
async fn awaiting_the_auth_cookie() -> (TestClient, JoinHandle<Option<&'static str>>) {
    let (mut client, served) = serve_with(Some(SECRET));
    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Transfer,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            name: "Hydrofin".into(),
            uuid: Uuid::from_u128(7),
        })
        .await;
    let request = client.expect::<login::ServerCookieRequestPacket>().await;
    assert_eq!(request.key, AuthCookie::KEY);
    (client, served)
}

#[tokio::test]
async fn a_packet_that_is_not_what_the_step_waits_for_is_refused() {
    // The step is what enforces the order, so a client that skips ahead is refused rather than
    // being let into a handler that would have to notice for itself.
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientEncryptionResponsePacket {
            shared_secret: Bytes::from_static(b"nonsense"),
            verify_token: Bytes::from_static(b"nonsense"),
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_step"));
}

#[tokio::test]
async fn nothing_but_the_cookie_may_arrive_while_one_is_awaited() {
    // What the read gate used to be for: the step that waits for a cookie waits for nothing else,
    // so a client cannot start a second login underneath an outstanding request.
    let (mut client, served) = awaiting_the_auth_cookie().await;

    client
        .send(login::ClientLoginStartPacket {
            name: "Impostor".into(),
            uuid: Uuid::from_u128(1),
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_step"));
}

#[tokio::test]
async fn a_cookie_for_a_key_nobody_asked_for_is_refused() {
    // The key is part of the step, so an answer to another question cannot be mistaken for the one
    // that was asked.
    let (mut client, served) = awaiting_the_auth_cookie().await;

    client
        .send(login::ClientCookieResponsePacket {
            key: SessionCookie::KEY.into(),
            payload: None,
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_cookie"));
}

#[tokio::test]
async fn a_cookie_nobody_asked_for_at_all_is_refused() {
    // Refused by the step rather than by the cookie handler, so the reason is `unexpected_step` and
    // not the `unexpected_cookie` above: no cookie was outstanding, so this is a packet arriving
    // where it does not belong rather than the wrong answer to a question that was asked.
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientCookieResponsePacket {
            key: SessionCookie::KEY.into(),
            payload: None,
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_step"));
}

#[tokio::test]
async fn a_ping_may_not_overtake_the_status_it_is_timing() {
    // The connection is held inside the status adapter, which is where a real one spends its time.
    // A client that pipelines its ping is refused rather than being answered out of order -- before
    // the step machine covered the adapter call, the pong was sent and the answer it was timing was
    // dropped on the floor.
    let (status, release) = HeldStatusAdapter::new();
    let (mut client, served) = serve_routes(
        routes_with(
            DynStatusAdapter::Held(status),
            DynAuthenticationAdapter::Disabled(DisabledAuthenticationAdapter::new()),
        ),
        None,
    );

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Status,
        })
        .await;
    client.send(status::ClientStatusRequestPacket).await;
    client
        .send(status::ClientPingRequestPacket { payload: 0x1234 })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_step"));
    release.release();
}

#[tokio::test]
async fn a_second_encryption_response_cannot_arrive_during_authentication() {
    // The session server call is the longest wait in the flow. A duplicate response let through it
    // would authenticate twice and, worse, switch the cipher a second time mid-stream.
    let profile = Profile {
        id: Uuid::from_u128(7),
        name: "Hydrofin".into(),
        properties: Vec::new(),
        profile_actions: Vec::new(),
    };
    let (authentication, release) = HeldAuthenticationAdapter::new(profile);
    let (mut client, served) = serve_routes(
        routes_with(
            DynStatusAdapter::Fixed(FixedStatusAdapter::new(
                None,
                versions::V26_3,
                versions::V1_20_5,
                versions::V26_3,
            )),
            DynAuthenticationAdapter::Held(authentication),
        ),
        None,
    );

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            name: "Hydrofin".into(),
            uuid: Uuid::from_u128(7),
        })
        .await;

    let encryption = client
        .expect::<login::ServerEncryptionRequestPacket>()
        .await;
    let public = crypto::KEY_PAIR.1.clone();
    let secret = b"0123456789abcdef";
    let response = login::ClientEncryptionResponsePacket {
        shared_secret: crypto::encrypt(&public, secret).expect("encrypts").into(),
        verify_token: crypto::encrypt(&public, &encryption.verify_token)
            .expect("encrypts")
            .into(),
    };

    // The first response starts the authentication, which is held. The second arrives while it is.
    client.send(response.clone()).await;
    client.encrypt(secret);
    client.send(response).await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("unexpected_step"));
    release.release();
}

#[tokio::test]
async fn a_client_older_than_the_transfer_packet_is_told_which_version_to_install() {
    // 1.20.4: the last version without the transfer packet, and the whole reason
    // `disconnect_unsupported` exists. It has to arrive as a login disconnect -- a dropped
    // connection is what the client reports as "connection reset", and nothing else.
    const V1_20_4: ProtocolVersion = ProtocolVersion::new(765);
    let (mut client, served) = serve();
    client.version = V1_20_4;

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: V1_20_4,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    // 1.20.4's login start is the same two fields under the same ID. It simply has no ID *in this
    // crate* below 1.20.5, so the frame is addressed by hand -- which is exactly what the client
    // does.
    client
        .send_as(
            0x00,
            login::ClientLoginStartPacket {
                name: "Hydrofin".into(),
                uuid: Uuid::from_u128(7),
            },
        )
        .await;

    // The reply is read raw for the same reason: the disconnect has no ID at 765 either, so
    // `expect` would be asserting against a table that does not cover this client.
    let frame = client
        .next_raw()
        .await
        .expect("an old client is answered rather than dropped");
    assert_eq!(frame.id, 0x00, "the login disconnect");
    let mut reader = Reader::new(frame.payload);
    reader.var_int("packet_id").expect("the ID leads");
    let disconnect =
        login::ServerDisconnectPacket::decode(&mut reader, V1_20_4).expect("the packet decodes");
    // The route's message table is empty, so the localization adapter hands back the key -- which
    // is still the assertion that matters: the client is told *this*, rather than dropped or sent
    // something meant for another situation.
    assert_eq!(disconnect.reason, "disconnect_unsupported");

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), None, "a clean ending");
}

#[tokio::test]
async fn the_client_is_only_sent_to_mojang_when_the_route_will_check_the_answer() {
    // `should_authenticate` is an instruction to the client, not a note to ourselves: when it is
    // set, the client posts to the session server and refuses to continue if that fails. So it has
    // to follow the route's adapter. A route that authenticates nobody and still demands a Mojang
    // session turns away exactly the players it was configured to let in.
    let profile = Profile {
        id: Uuid::from_u128(7),
        name: "Hydrofin".into(),
        properties: Vec::new(),
        profile_actions: Vec::new(),
    };
    let (authentication, release) = HeldAuthenticationAdapter::new(profile);
    let (mut client, served) = serve_routes(
        routes_with(
            DynStatusAdapter::Fixed(FixedStatusAdapter::new(
                None,
                versions::V26_3,
                versions::V1_20_5,
                versions::V26_3,
            )),
            DynAuthenticationAdapter::Held(authentication),
        ),
        None,
    );

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_3,
            server_address: HOST.into(),
            server_port: 25_565,
            next_state: Intent::Login,
        })
        .await;
    client
        .send(login::ClientLoginStartPacket {
            name: "Hydrofin".into(),
            uuid: Uuid::from_u128(7),
        })
        .await;

    let encryption = client
        .expect::<login::ServerEncryptionRequestPacket>()
        .await;
    assert!(
        encryption.should_authenticate,
        "an adapter that consults Mojang has to have the client do so first",
    );

    // The counterpart is asserted in `a_login_runs_to_the_transfer_packet`, whose route disables
    // authentication and therefore must not set the flag.
    release.release();
    drop(client);
    let _ = served.await;
}
