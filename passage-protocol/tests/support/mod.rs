//! A minimal in-process Passage deployment plus a Minecraft status-ping client.
//!
//! The harness boots a real [`Listener`] on a loopback port with the built-in fixed adapters, so
//! tests exercise the same accept -> handshake -> status -> pong path a Minecraft client walks. No
//! adapter performs I/O, which keeps the measurements focused on what Passage itself costs per
//! connection.

#![allow(dead_code)]

use futures::{SinkExt, StreamExt};
use passage_adapters::{
    DisabledAuthenticationAdapter, FixedDiscoveryAdapter, FixedLocalizationAdapter,
    FixedStatusAdapter, ServerPlayers, ServerStatus, ServerVersion,
};
use passage_packets::codec::PacketCodec;
use passage_packets::handshake::serverbound::HandshakePacket;
use passage_packets::status::clientbound::{PongPacket, StatusResponsePacket};
use passage_packets::status::serverbound::{PingPacket, StatusRequestPacket};
use passage_packets::{State, VarInt};
use passage_protocol::config::Config;
use passage_protocol::listener::Listener;
use passage_protocol::routes::{Route, Routes};
use regex::Regex;
use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;

/// The hostname every harness client sends in its handshake.
pub const HOSTNAME: &str = "harness.example.net";

/// The protocol version every harness client claims to speak (1.21.5).
pub const PROTOCOL_VERSION: VarInt = 770;

/// The adapter set used by the harness, spelled out because [`Routes`] is generic over it.
type HarnessRoutes = Routes<
    FixedStatusAdapter,
    FixedDiscoveryAdapter,
    DisabledAuthenticationAdapter,
    FixedLocalizationAdapter,
>;

/// A running listener bound to a loopback port.
pub struct Harness {
    /// The address the listener accepts connections on.
    pub address: SocketAddr,
    stop: CancellationToken,
    listener: JoinHandle<()>,
}

impl Harness {
    /// Boots a listener with the given protocol configuration and waits until it accepts
    /// connections.
    pub async fn start(config: Config) -> Self {
        let address = free_address();
        let stop = CancellationToken::new();

        let listener_stop = stop.clone();
        let listener = tokio::spawn(async move {
            let mut listener = Listener::new(routes(), None, config);
            listener
                .listen(address, listener_stop)
                .await
                .expect("harness listener failed");
        });

        // The listener binds asynchronously, so probe until it serves a full status ping. Probing
        // with a complete exchange rather than a bare connect means every per-connection resource
        // the startup itself allocates is already accounted for once `start` returns, which keeps
        // task-count baselines taken by the caller exact.
        for _ in 0..100 {
            if status_ping(address).await.is_ok() {
                return Self {
                    address,
                    stop,
                    listener,
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("harness listener did not come up on {address}");
    }

    /// Boots a listener with the default protocol configuration.
    pub async fn start_default() -> Self {
        Self::start(Config::default()).await
    }

    /// Performs one complete status ping: handshake, status request, response, ping, pong.
    ///
    /// Returns the raw JSON body of the status response.
    pub async fn status_ping(&self) -> Result<String, Box<dyn Error + Send + Sync>> {
        status_ping(self.address).await
    }

    /// Stops the listener and waits for it to drain.
    pub async fn shutdown(self) {
        self.stop.cancel();
        let _ = self.listener.await;
    }
}

/// Performs one complete status ping against `address`.
pub async fn status_ping(address: SocketAddr) -> Result<String, Box<dyn Error + Send + Sync>> {
    let stream = TcpStream::connect(address).await?;
    stream.set_nodelay(true)?;
    let mut framed = Framed::new(stream, PacketCodec::new(1 << 21));

    framed
        .send(HandshakePacket {
            protocol_version: PROTOCOL_VERSION,
            server_address: HOSTNAME.to_owned(),
            server_port: 25565,
            next_state: State::Status,
        })
        .await?;
    framed.send(StatusRequestPacket).await?;

    let frame = framed
        .next()
        .await
        .ok_or("closed before status response")??;
    let response: StatusResponsePacket = frame.try_into()?;

    framed
        .send(PingPacket {
            payload: 0x5041_5353,
        })
        .await?;
    let frame = framed.next().await.ok_or("closed before pong")??;
    let pong: PongPacket = frame.try_into()?;
    if pong.payload != 0x5041_5353 {
        return Err("pong payload did not match ping payload".into());
    }

    Ok(response.body)
}

/// Opens a TCP connection and closes it again without speaking any Minecraft protocol.
///
/// Used as the control group: it exercises accept and teardown, but no packet handling.
pub async fn connect_only(address: SocketAddr) -> Result<(), Box<dyn Error + Send + Sync>> {
    let stream = TcpStream::connect(address).await?;
    drop(stream);
    Ok(())
}

/// Builds the single-route adapter set used by the harness.
fn routes() -> HarnessRoutes {
    let status = ServerStatus {
        version: ServerVersion {
            name: "Passage Harness".to_owned(),
            protocol: PROTOCOL_VERSION,
        },
        players: Some(ServerPlayers {
            online: 0,
            max: 100,
            sample: None,
        }),
        description: Some(
            serde_json::value::RawValue::from_string(r#"{"text":"harness"}"#.to_owned()).unwrap(),
        ),
        favicon: None,
        enforces_secure_chat: Some(false),
    };

    let route = Route {
        hostname: Regex::new(&format!("^{}$", regex::escape(HOSTNAME))).unwrap(),
        status_adapter: FixedStatusAdapter::new(Some(status), PROTOCOL_VERSION, 0, VarInt::MAX),
        discovery_adapter: FixedDiscoveryAdapter::new(vec![]),
        authentication_adapter: DisabledAuthenticationAdapter::new(),
        localization_adapter: FixedLocalizationAdapter::new("en".to_owned(), HashMap::new(), true),
    };

    vec![std::sync::Arc::new(route)].into()
}

/// Reserves a loopback address by binding port zero and releasing it again.
///
/// There is a small race between release and re-bind, which is acceptable for tests and avoids
/// hard-coding ports that collide when tests run in parallel.
fn free_address() -> SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("no free loopback port")
        .local_addr()
        .expect("bound socket has no address")
}

/// Reads the resident set size of the current process in kilobytes.
///
/// Returns `None` on platforms without a procfs.
pub fn resident_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

/// Returns the number of tasks currently alive in the ambient tokio runtime.
pub fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}
