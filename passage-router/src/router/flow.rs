//! End-to-end tests for the handlers, driven the way a Minecraft client drives them.
//!
//! The test is the client: it puts real frames on a socket, switches to the cipher when the
//! protocol says to, and reads what comes back. Nothing here reaches into the handlers, so what is
//! proven is the conversation rather than the code that produces it.

use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::held::{HeldAuthenticationAdapter, HeldStatusAdapter};
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::cookie::{AuthCookie, Cookie, SessionCookie};
use crate::crypto;
use crate::router::state::State;
use crate::router::{DynRoute, Passage};
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
use passage_core::wire::{Bytes, Options as WireOptions, Reader};
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

/// One route, answered entirely from memory: no session server, no discovery service.
fn routes() -> Arc<[Arc<DynRoute>]> {
    routes_with(
        DynStatusAdapter::Fixed(FixedStatusAdapter::new(
            None,
            versions::V26_1,
            versions::V1_20_5,
            versions::V26_1,
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
        status_adapter,
        discovery_adapter: DynDiscoveryActionAdapter::FixedDiscovery(FixedDiscoveryAdapter::new(
            vec![target],
        )),
        authentication_adapter,
        localization_adapter: DynLocalizationAdapter::Fixed(FixedLocalizationAdapter::new(
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
    let router = Passage::new(Arc::clone(&routes), secret.clone(), 3_600)
        .router()
        .expect("the router builds");

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
            version: versions::V26_1,
            options,
        }
    }

    /// Sends a packet, encoded for the version this client speaks.
    async fn send<P: Packet>(&mut self, packet: P) {
        let frame = Frame::of(&packet, self.version, self.options).expect("encodes");
        self.framed.send(frame).await.expect("writes");
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
async fn a_status_ping_is_answered_and_the_connection_closed() {
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_1,
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
            protocol_version: versions::V26_1,
            server_address: "elsewhere.example".into(),
            server_port: 25_565,
            next_state: Intent::Status,
        })
        .await;

    client.expect_eof().await;
    assert_eq!(served.await.expect("no panic"), Some("no_route"));
}

#[tokio::test]
async fn a_login_runs_to_the_transfer_packet() {
    let (mut client, served) = serve();

    // Handshake and login start, in the clear.
    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_1,
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
    assert!(encryption.should_authenticate, "no auth cookie was offered");

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
async fn a_transfer_with_a_valid_auth_cookie_skips_authentication() {
    let (mut client, served) = serve_with(Some(SECRET));

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_1,
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
            protocol_version: versions::V26_1,
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
            protocol_version: versions::V26_1,
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
    let (mut client, served) = serve();

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_1,
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
    assert_eq!(served.await.expect("no panic"), Some("unexpected_cookie"));
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
            protocol_version: versions::V26_1,
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
                versions::V26_1,
                versions::V1_20_5,
                versions::V26_1,
            )),
            DynAuthenticationAdapter::Held(authentication),
        ),
        None,
    );

    client
        .send(handshake::ClientIntentionPacket {
            protocol_version: versions::V26_1,
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
