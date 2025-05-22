use async_trait::async_trait;
use packets::configuration::clientbound as conf_out;
use packets::configuration::serverbound as conf_in;
use packets::handshake::serverbound as hand_in;
use packets::login::clientbound as login_out;
use packets::login::serverbound as login_in;
use packets::status::clientbound as status_out;
use packets::status::serverbound as status_in;
use packets::{AsyncReadPacket, AsyncWritePacket, ResourcePackResult, State};
use passage::adapter::resourcepack::fixed::FixedResourcePackSupplier;
use passage::adapter::resourcepack::none::NoneResourcePackSupplier;
use passage::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use passage::adapter::status::StatusSupplier;
use passage::adapter::status::none::NoneStatusSupplier;
use passage::adapter::target_selection::TargetSelector;
use passage::adapter::target_selection::none::NoneTargetSelector;
use passage::adapter::target_strategy::TargetSelectorStrategy;
use passage::adapter::target_strategy::none::NoneTargetSelectorStrategy;
use passage::authentication;
use passage::cipher_stream::CipherStream;
use passage::connection::{AUTH_COOKIE_KEY, AuthCookie, Connection, Error};
use passage::mojang::{AuthResponse, Mojang};
use rand::rngs::OsRng;
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::uuid;

#[derive(Default)]
struct MojangMock {
    pub response: AuthResponse,
}

impl MojangMock {
    pub fn new(response: AuthResponse) -> Self {
        Self { response }
    }
}

#[async_trait]
impl Mojang for MojangMock {
    async fn authenticate(
        &self,
        _username: &str,
        _shared_secret: &[u8],
        _server_id: &str,
        _encoded_public: &[u8],
    ) -> Result<AuthResponse, reqwest::Error> {
        Ok(self.response.clone())
    }
}

pub fn encrypt(key: &RsaPublicKey, value: &[u8]) -> Vec<u8> {
    key.encrypt(&mut OsRng, Pkcs1v15Encrypt, value)
        .expect("encrypt failed")
}

#[tokio::test]
async fn simulate_handshake() {
    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(NoneResourcePackSupplier);

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::default()),
        None,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let res = server.listen(client_address).await;
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

#[tokio::test]
async fn simulate_status() {
    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(NoneResourcePackSupplier);

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::default()),
        None,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        server
            .listen(client_address)
            .await
            .expect("server listen failed");
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

#[tokio::test]
async fn simulate_transfer_no_configuration() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(NoneResourcePackSupplier);

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::default()),
        Some(auth_secret.clone()),
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen(client_address).await;
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
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(authentication::sign(&auth_payload, &auth_secret)),
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

    let (encryptor, decryptor) =
        authentication::create_ciphers(shared_secret).expect("create ciphers failed");
    let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));

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

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test]
async fn simulate_login_no_configuration() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(NoneResourcePackSupplier);

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::new(AuthResponse {
            id: user_id,
            name: user_name.clone(),
        })),
        None,
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen(client_address).await;
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

    let (encryptor, decryptor) =
        authentication::create_ciphers(shared_secret).expect("create ciphers failed");
    let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));

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

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test]
async fn sends_keep_alive() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(
        FixedResourcePackSupplier::new(vec![Resourcepack::default()]),
    );

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::default()),
        Some(auth_secret.clone()),
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        let result = server.listen(client_address).await;
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
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(authentication::sign(&auth_payload, &auth_secret)),
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

    let (encryptor, decryptor) =
        authentication::create_ciphers(shared_secret).expect("create ciphers failed");
    let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));

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

    // accept resource pack but wait
    let add_pack: conf_out::AddResourcePackPacket = client_stream
        .read_packet()
        .await
        .expect("add resource pack packet read failed");

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(10)).await;
    let _: conf_out::KeepAlivePacket = client_stream
        .read_packet()
        .await
        .expect("keep-alive packet read failed");

    client_stream
        .write_packet(conf_in::ResourcePackResponsePacket {
            uuid: add_pack.uuid,
            result: ResourcePackResult::Success,
        })
        .await
        .expect("send resource pack response packet failed");

    // disconnect as no target configured
    let _disconnect_packet: conf_out::DisconnectPacket = client_stream
        .read_packet()
        .await
        .expect("disconnect packet read failed");

    // wait for the server to finish
    server.await.expect("server run failed");
}

#[tokio::test]
async fn no_respond_keep_alive() {
    let shared_secret = b"verysecuresecret";
    let user_name = "Hydrofin".to_owned();
    let user_id = uuid!("09879557-e479-45a9-b434-a56377674627");

    // create stream
    let auth_secret = b"secret".to_vec();
    let client_address = SocketAddr::from_str("127.0.0.1:25564").expect("invalid address");
    let (mut client_stream, server_stream) = tokio::io::duplex(1024);

    // build supplier
    let status_supplier: Arc<dyn StatusSupplier> = Arc::new(NoneStatusSupplier);
    let strategy: Arc<dyn TargetSelectorStrategy> = Arc::new(NoneTargetSelectorStrategy);
    let target_selector: Arc<dyn TargetSelector> = Arc::new(NoneTargetSelector::new(strategy));
    let resourcepack_supplier: Arc<dyn ResourcepackSupplier> = Arc::new(
        FixedResourcePackSupplier::new(vec![Resourcepack::default()]),
    );

    // build connection
    let mut server = Connection::new(
        server_stream,
        Arc::clone(&status_supplier),
        Arc::clone(&target_selector),
        Arc::clone(&resourcepack_supplier),
        Arc::new(MojangMock::default()),
        Some(auth_secret.clone()),
    );

    // start the server in its own thread
    let server = tokio::spawn(async move {
        server
            .listen(client_address)
            .await
            .expect("server listen failed");
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
    })
    .expect("auth cookie serialization failed");

    client_stream
        .write_packet(login_in::CookieResponsePacket {
            key: cookie_request_packet.key,
            payload: Some(authentication::sign(&auth_payload, &auth_secret)),
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

    let (encryptor, decryptor) =
        authentication::create_ciphers(shared_secret).expect("create ciphers failed");
    let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));

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

    // accept resource pack but wait
    let _: conf_out::AddResourcePackPacket = client_stream
        .read_packet()
        .await
        .expect("add resource pack packet read failed");

    // advance multiple times to ensure keep-alive is sent multiple times
    tokio::time::pause();
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
