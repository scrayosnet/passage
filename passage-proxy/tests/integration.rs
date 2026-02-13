use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{FixedDiscoveryAdapter, FixedStatusAdapter, FixedStrategyAdapter, Target};
use passage_packets::configuration::clientbound as conf_out;
use passage_packets::configuration::serverbound as conf_in;
use passage_packets::handshake::serverbound as hand_in;
use passage_packets::login::clientbound as login_out;
use passage_packets::login::serverbound as login_in;
use passage_packets::status::clientbound as status_out;
use passage_packets::status::serverbound as status_in;
use passage_packets::{
    AsyncReadPacket, AsyncWritePacket, ChatMode, DisplayedSkinParts, MainHand, ParticleStatus,
    State,
};
use passage_proxy::Error;
use passage_proxy::connection::Connection;
use passage_proxy::cookie::{AUTH_COOKIE_KEY, AuthCookie, SESSION_COOKIE_KEY, SessionCookie, sign};
use passage_proxy::crypto::stream::CipherStream;
use passage_proxy::localization::Localization;
use passage_proxy::mojang::{Mojang, Profile};
use proxy_header::ParseConfig;
use proxy_header::io::ProxiedStream;
use rand::rngs::SysRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::rand_core::UnwrapErr;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use uuid::uuid;

#[derive(Default)]
struct MojangMock {
    pub response: Profile,
}

impl MojangMock {
    pub fn new(response: Profile) -> Self {
        Self { response }
    }
}

impl Mojang for MojangMock {
    async fn authenticate(
        &self,
        _username: &str,
        _shared_secret: &[u8],
        _server_id: &str,
        _encoded_public: &[u8],
    ) -> Result<Profile, reqwest::Error> {
        Ok(self.response.clone())
    }
}

pub fn encrypt(key: &RsaPublicKey, value: &[u8]) -> Vec<u8> {
    key.encrypt(&mut UnwrapErr(SysRng), Pkcs1v15Encrypt, value)
        .expect("encrypt failed")
}

#[derive(Debug)]
struct SlowDiscoveryAdapter {
    duration: Duration,
}

impl SlowDiscoveryAdapter {
    pub fn new(seconds: u64) -> Self {
        Self {
            duration: Duration::from_secs(seconds),
        }
    }
}

impl DiscoveryAdapter for SlowDiscoveryAdapter {
    async fn discover(&self) -> passage_adapters::Result<Vec<Target>> {
        tokio::time::sleep(self.duration).await;
        Ok(vec![])
    }
}

#[tokio::test(start_paused = true)]
async fn simulate_handshake() {
    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        None,
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let res = server.listen().await;
        match res {
            Err(Error::ConnectionClosed(_)) => {}
            other => panic!("expected connection closed, got {:?}", other),
        }
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Status,
        })
        .await
        .expect("send handshake failed");

    // simulate connection closed after the handshake packet
    drop(client_stream);

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn simulate_status() {
    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        None,
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        server.listen().await.expect("server listen failed");
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Status,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(status_in::StatusRequestPacket)
        .await
        .expect("send status request failed");

    let status_response_packet: status_out::StatusResponsePacket = client_stream
        .read_packet()
        .await
        .expect("status response packet read failed");
    assert_eq!(status_response_packet.body, "null");

    client_stream
        .write_packet(status_in::PingPacket { payload: 42 })
        .await
        .expect("send ping request failed");

    let pong_packet: status_out::PongPacket = client_stream
        .read_packet()
        .await
        .expect("pong packet read failed");
    assert_eq!(pong_packet.payload, 42);

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn simulate_transfer_no_configuration() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        Some(auth_secret.clone()),
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen().await;
        match result {
            Err(Error::NoTargetFound) => {}
            other => panic!("expected no target found, got {:?}", other),
        }
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Transfer,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(login_in::LoginStartPacket {
            user_name: user_name.clone(),
            user_id,
        })
        .await
        .expect("send login start failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("session cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, SESSION_COOKIE_KEY);

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(
                serde_json::to_vec(&SessionCookie {
                    id: Default::default(),
                    server_address: "".to_string(),
                    server_port: 0,
                })
                .expect("session cookie serialization failed"),
            ),
        })
        .await
        .expect("send session cookie response failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, AUTH_COOKIE_KEY);

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time error")
        .as_secs();
    let auth_payload = serde_json::to_vec(&AuthCookie {
        timestamp: now_secs,
        client_addr: client_address,
        user_name: user_name.clone(),
        user_id,
        target: None,
        profile_properties: vec![],
        extra: Default::default(),
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(sign(&auth_payload, &auth_secret)),
        })
        .await
        .expect("send cookie response failed");

    let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
        .read_packet()
        .await
        .expect("encryption request packet read failed");
    assert!(!encryption_request_packet.should_authenticate);

    let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
        .expect("public key deserialization failed");
    let enc_shared_secret = encrypt(&pub_key, shared_secret);
    let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
    client_stream
        .write_packet(login_in::EncryptionResponsePacket {
            shared_secret: enc_shared_secret,
            verify_token: enc_verify_token,
        })
        .await
        .expect("send encryption response failed");

    let mut client_stream =
        CipherStream::from_secret(client_stream, shared_secret).expect("create ciphers failed");

    let login_success_packet: login_out::LoginSuccessPacket = client_stream
        .read_packet()
        .await
        .expect("login success packet read failed");
    assert_eq!(login_success_packet.user_name, user_name);
    assert_eq!(login_success_packet.user_id, user_id);

    client_stream
        .write_packet(login_in::LoginAcknowledgedPacket)
        .await
        .expect("send login acknowledged packet failed");

    client_stream
        .write_packet(conf_in::ClientInformationPacket {
            locale: "de_DE".to_string(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: false,
            displayed_skin_parts: DisplayedSkinParts(0),
            main_hand: MainHand::Left,
            enable_text_filtering: false,
            allow_server_listing: false,
            particle_status: ParticleStatus::All,
        })
        .await
        .expect("send client information packet failed");

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn simulate_slow_transfer_no_configuration() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(SlowDiscoveryAdapter::new(29));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        Some(auth_secret.clone()),
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen().await;
        match result {
            Err(Error::NoTargetFound) => {}
            other => panic!("expected no target found, got {:?}", other),
        }
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Transfer,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(login_in::LoginStartPacket {
            user_name: user_name.clone(),
            user_id,
        })
        .await
        .expect("send login start failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("session cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, SESSION_COOKIE_KEY);

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(
                serde_json::to_vec(&SessionCookie {
                    id: Default::default(),
                    server_address: "".to_string(),
                    server_port: 0,
                })
                .expect("session cookie serialization failed"),
            ),
        })
        .await
        .expect("send session cookie response failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, AUTH_COOKIE_KEY);

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time error")
        .as_secs();
    let auth_payload = serde_json::to_vec(&AuthCookie {
        timestamp: now_secs,
        client_addr: client_address,
        user_name: user_name.clone(),
        user_id,
        target: None,
        profile_properties: vec![],
        extra: Default::default(),
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(sign(&auth_payload, &auth_secret)),
        })
        .await
        .expect("send cookie response failed");

    let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
        .read_packet()
        .await
        .expect("encryption request packet read failed");
    assert!(!encryption_request_packet.should_authenticate);

    let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
        .expect("public key deserialization failed");
    let enc_shared_secret = encrypt(&pub_key, shared_secret);
    let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
    client_stream
        .write_packet(login_in::EncryptionResponsePacket {
            shared_secret: enc_shared_secret,
            verify_token: enc_verify_token,
        })
        .await
        .expect("send encryption response failed");

    let mut client_stream =
        CipherStream::from_secret(client_stream, shared_secret).expect("create ciphers failed");

    let login_success_packet: login_out::LoginSuccessPacket = client_stream
        .read_packet()
        .await
        .expect("login success packet read failed");
    assert_eq!(login_success_packet.user_name, user_name);
    assert_eq!(login_success_packet.user_id, user_id);

    client_stream
        .write_packet(login_in::LoginAcknowledgedPacket)
        .await
        .expect("send login acknowledged packet failed");

    client_stream
        .write_packet(conf_in::ClientInformationPacket {
            locale: "de_DE".to_string(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: false,
            displayed_skin_parts: DisplayedSkinParts(0),
            main_hand: MainHand::Left,
            enable_text_filtering: false,
            allow_server_listing: false,
            particle_status: ParticleStatus::All,
        })
        .await
        .expect("send client information packet failed");

    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");

    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn simulate_login_no_configuration() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::new(Profile {
        id: user_id,
        name: user_name.clone(),
        properties: vec![],
        profile_actions: vec![],
    }));
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        None,
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen().await;
        match result {
            Err(Error::NoTargetFound) => {}
            other => panic!("expected no target found, got {:?}", other),
        }
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Login,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(login_in::LoginStartPacket {
            user_name: user_name.clone(),
            user_id,
        })
        .await
        .expect("send login start failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("session cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, SESSION_COOKIE_KEY);

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(
                serde_json::to_vec(&SessionCookie {
                    id: Default::default(),
                    server_address: "".to_string(),
                    server_port: 0,
                })
                .expect("session cookie serialization failed"),
            ),
        })
        .await
        .expect("send session cookie response failed");

    let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
        .read_packet()
        .await
        .expect("encryption request packet read failed");
    assert!(encryption_request_packet.should_authenticate);

    let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
        .expect("public key deserialization failed");
    let enc_shared_secret = encrypt(&pub_key, shared_secret);
    let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
    client_stream
        .write_packet(login_in::EncryptionResponsePacket {
            shared_secret: enc_shared_secret,
            verify_token: enc_verify_token,
        })
        .await
        .expect("send encryption response failed");

    let mut client_stream =
        CipherStream::from_secret(client_stream, shared_secret).expect("create ciphers failed");

    let login_success_packet: login_out::LoginSuccessPacket = client_stream
        .read_packet()
        .await
        .expect("login success packet read failed");
    assert_eq!(login_success_packet.user_name, user_name);
    assert_eq!(login_success_packet.user_id, user_id);

    client_stream
        .write_packet(login_in::LoginAcknowledgedPacket)
        .await
        .expect("send login acknowledged packet failed");

    client_stream
        .write_packet(conf_in::ClientInformationPacket {
            locale: "de_DE".to_string(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: false,
            displayed_skin_parts: DisplayedSkinParts(0),
            main_hand: MainHand::Left,
            enable_text_filtering: false,
            allow_server_listing: false,
            particle_status: ParticleStatus::All,
        })
        .await
        .expect("send client information packet failed");

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn sends_keep_alive() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        Some(auth_secret.clone()),
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen().await;
        match result {
            Err(Error::NoTargetFound) => {}
            other => panic!("expected no target found, got {:?}", other),
        }
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Transfer,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(login_in::LoginStartPacket {
            user_name: user_name.clone(),
            user_id,
        })
        .await
        .expect("send login start failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("session cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, SESSION_COOKIE_KEY);

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(
                serde_json::to_vec(&SessionCookie {
                    id: Default::default(),
                    server_address: "".to_string(),
                    server_port: 0,
                })
                .expect("session cookie serialization failed"),
            ),
        })
        .await
        .expect("send session cookie response failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, AUTH_COOKIE_KEY);

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time error")
        .as_secs();
    let auth_payload = serde_json::to_vec(&AuthCookie {
        timestamp: now_secs,
        client_addr: client_address,
        user_name: user_name.clone(),
        user_id,
        target: None,
        profile_properties: vec![],
        extra: Default::default(),
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(sign(&auth_payload, &auth_secret)),
        })
        .await
        .expect("send cookie response failed");

    let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
        .read_packet()
        .await
        .expect("encryption request packet read failed");
    assert!(!encryption_request_packet.should_authenticate);

    let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
        .expect("public key deserialization failed");
    let enc_shared_secret = encrypt(&pub_key, shared_secret);
    let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
    client_stream
        .write_packet(login_in::EncryptionResponsePacket {
            shared_secret: enc_shared_secret,
            verify_token: enc_verify_token,
        })
        .await
        .expect("send encryption response failed");

    let mut client_stream =
        CipherStream::from_secret(client_stream, shared_secret).expect("create ciphers failed");

    let login_success_packet: login_out::LoginSuccessPacket = client_stream
        .read_packet()
        .await
        .expect("login success packet read failed");
    assert_eq!(login_success_packet.user_name, user_name);
    assert_eq!(login_success_packet.user_id, user_id);

    client_stream
        .write_packet(login_in::LoginAcknowledgedPacket)
        .await
        .expect("send login acknowledged packet failed");

    tokio::time::advance(Duration::from_secs(10)).await;
    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");

    client_stream
        .write_packet(conf_in::ClientInformationPacket {
            locale: "de_DE".to_string(),
            view_distance: 10,
            chat_mode: ChatMode::Enabled,
            chat_colors: false,
            displayed_skin_parts: DisplayedSkinParts(0),
            main_hand: MainHand::Left,
            enable_text_filtering: false,
            allow_server_listing: false,
            particle_status: ParticleStatus::All,
        })
        .await
        .expect("send client information packet failed");

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test(start_paused = true)]
async fn no_respond_keep_alive() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_adapter = Arc::new(FixedStatusAdapter::new(None, 0, 0, 0));
    let strategy_adapter = Arc::new(FixedStrategyAdapter::new());
    let discovery_adapter = Arc::new(FixedDiscoveryAdapter::new(vec![]));
    let mojang = Arc::new(MojangMock::default());
    let localization = Arc::new(Localization::default());

    // build connection
    let mut server = Connection::new(
        server_stream,
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
        Some(auth_secret.clone()),
        client_address,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        server.listen().await.expect("server listen failed");
    });

    // simulate client
    client_stream
        .write_packet(hand_in::HandshakePacket {
            protocol_version: 0,
            server_address: "".to_string(),
            server_port: 0,
            next_state: State::Transfer,
        })
        .await
        .expect("send handshake failed");

    client_stream
        .write_packet(login_in::LoginStartPacket {
            user_name: user_name.clone(),
            user_id,
        })
        .await
        .expect("send login start failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("session cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, SESSION_COOKIE_KEY);

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(
                serde_json::to_vec(&SessionCookie {
                    id: Default::default(),
                    server_address: "".to_string(),
                    server_port: 0,
                })
                .expect("session cookie serialization failed"),
            ),
        })
        .await
        .expect("send session cookie response failed");

    let cookie_request_packet: login_out::CookieRequestPacket = client_stream
        .read_packet()
        .await
        .expect("cookie request packet read failed");
    assert_eq!(&cookie_request_packet.key, AUTH_COOKIE_KEY);

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time error")
        .as_secs();
    let auth_payload = serde_json::to_vec(&AuthCookie {
        timestamp: now_secs,
        client_addr: client_address,
        user_name: user_name.clone(),
        user_id,
        target: None,
        profile_properties: vec![],
        extra: Default::default(),
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(sign(&auth_payload, &auth_secret)),
        })
        .await
        .expect("send cookie response failed");

    let encryption_request_packet: login_out::EncryptionRequestPacket = client_stream
        .read_packet()
        .await
        .expect("encryption request packet read failed");
    assert!(!encryption_request_packet.should_authenticate);

    let pub_key = RsaPublicKey::from_public_key_der(&encryption_request_packet.public_key)
        .expect("public key deserialization failed");
    let enc_shared_secret = encrypt(&pub_key, shared_secret);
    let enc_verify_token = encrypt(&pub_key, &encryption_request_packet.verify_token);
    client_stream
        .write_packet(login_in::EncryptionResponsePacket {
            shared_secret: enc_shared_secret,
            verify_token: enc_verify_token,
        })
        .await
        .expect("send encryption response failed");

    let mut client_stream =
        CipherStream::from_secret(client_stream, shared_secret).expect("create ciphers failed");

    let login_success_packet: login_out::LoginSuccessPacket = client_stream
        .read_packet()
        .await
        .expect("login success packet read failed");
    assert_eq!(login_success_packet.user_name, user_name);
    assert_eq!(login_success_packet.user_id, user_id);

    client_stream
        .write_packet(login_in::LoginAcknowledgedPacket)
        .await
        .expect("send login acknowledged packet failed");

    // advance multiple times to ensure keep-alive is sent multiple times
    tokio::time::advance(Duration::from_secs(10)).await;
    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");
    tokio::time::advance(Duration::from_secs(10)).await;
    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");
    tokio::time::advance(Duration::from_secs(10)).await;

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    assert!(server.await.is_err());
}

#[tokio::test]
async fn test_proxy_protocol_v1_ipv4() {
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // Write PROXY protocol v1 header
    let proxy_header = b"PROXY TCP4 192.168.1.100 10.0.0.1 12345 25565\r\n";
    client_stream
        .write_all(proxy_header)
        .await
        .expect("write proxy header failed");

    // read and parse the header on the server side
    let server_stream = ProxiedStream::create_from_tokio(server_stream, ParseConfig::default())
        .await
        .expect("proxy stream failed");

    let proxied = server_stream
        .proxy_header()
        .proxied_address()
        .expect("proxy address failed");
    assert_eq!(proxied.source.to_string(), "192.168.1.100:12345");
    assert_eq!(proxied.destination.to_string(), "10.0.0.1:25565");
    assert_eq!(proxied.protocol, proxy_header::Protocol::Stream);
}

#[tokio::test]
async fn test_proxy_protocol_v1_ipv6() {
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // Write PROXY protocol v1 header with IPv6
    // Note: IPv6 addresses in PROXY protocol v1 should not have brackets
    let proxy_header = b"PROXY TCP6 2001:0db8:0000:0000:0000:0000:0000:0001 2001:0db8:0000:0000:0000:0000:0000:0002 54321 25565\r\n";
    client_stream
        .write_all(proxy_header)
        .await
        .expect("write proxy header failed");

    // read and parse the header on the server side
    let server_stream = ProxiedStream::create_from_tokio(server_stream, ParseConfig::default())
        .await
        .expect("proxy stream failed");

    let proxied = server_stream
        .proxy_header()
        .proxied_address()
        .expect("proxy address failed");
    assert_eq!(proxied.source.to_string(), "[2001:db8::1]:54321");
    assert_eq!(proxied.destination.to_string(), "[2001:db8::2]:25565");
    assert_eq!(proxied.protocol, proxy_header::Protocol::Stream);
}

#[tokio::test]
async fn test_proxy_protocol_v2_ipv4() {
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // Write PROXY protocol v2 header for IPv4
    let mut proxy_header = vec![
        0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A, // signature
        0x21, // version 2, command PROXY
        0x11, // AF_INET (IPv4), SOCK_STREAM (TCP)
        0x00, 0x0C, // address length = 12 bytes
    ];
    // Source IP: 203.0.113.5
    proxy_header.extend_from_slice(&[203, 0, 113, 5]);
    // Dest IP: 198.51.100.1
    proxy_header.extend_from_slice(&[198, 51, 100, 1]);
    // Source port: 45678
    proxy_header.extend_from_slice(&45678u16.to_be_bytes());
    // Dest port: 25565
    proxy_header.extend_from_slice(&25565u16.to_be_bytes());

    client_stream
        .write_all(&proxy_header)
        .await
        .expect("write proxy header failed");

    // read and parse the header on the server side
    let server_stream = ProxiedStream::create_from_tokio(server_stream, ParseConfig::default())
        .await
        .expect("proxy stream failed");

    let proxied = server_stream
        .proxy_header()
        .proxied_address()
        .expect("proxy address failed");
    assert_eq!(proxied.source.to_string(), "203.0.113.5:45678");
    assert_eq!(proxied.destination.to_string(), "198.51.100.1:25565");
    assert_eq!(proxied.protocol, proxy_header::Protocol::Stream);
}

#[tokio::test]
async fn test_proxy_protocol_v2_ipv6() {
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // Write PROXY protocol v2 header for IPv6
    let mut proxy_header = vec![
        0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A, // signature
        0x21, // version 2, command PROXY
        0x21, // AF_INET6 (IPv6), SOCK_STREAM (TCP)
        0x00, 0x24, // address length = 36 bytes
    ];
    // Source IP: 2001:db8::cafe:beef
    proxy_header.extend_from_slice(&[
        0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xca, 0xfe, 0xbe,
        0xef,
    ]);
    // Dest IP: 2001:db8::1
    proxy_header.extend_from_slice(&[
        0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]);
    // Source port: 54321
    proxy_header.extend_from_slice(&54321u16.to_be_bytes());
    // Dest port: 25565
    proxy_header.extend_from_slice(&25565u16.to_be_bytes());

    client_stream
        .write_all(&proxy_header)
        .await
        .expect("write proxy header failed");

    // read and parse the header on the server side
    let server_stream = ProxiedStream::create_from_tokio(server_stream, ParseConfig::default())
        .await
        .expect("proxy stream failed");

    let proxied = server_stream
        .proxy_header()
        .proxied_address()
        .expect("proxy address failed");
    assert_eq!(proxied.source.to_string(), "[2001:db8::cafe:beef]:54321");
    assert_eq!(proxied.destination.to_string(), "[2001:db8::1]:25565");
    assert_eq!(proxied.protocol, proxy_header::Protocol::Stream);
}

#[tokio::test]
async fn test_proxy_protocol_invalid_header() {
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // Write invalid data
    let invalid_header = b"INVALID DATA\r\n";
    client_stream
        .write_all(invalid_header)
        .await
        .expect("write invalid header failed");

    // read and parse the header on the server side
    let server_stream =
        ProxiedStream::create_from_tokio(server_stream, ParseConfig::default()).await;
    assert!(server_stream.is_err());
}
