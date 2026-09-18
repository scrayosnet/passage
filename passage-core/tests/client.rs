use passage_core::client::{Client, Preconnected};
use passage_core::connection::DispatchError;
use passage_core::router::Router;
use passage_core::server::Server;
use passage_core::wire::{Reader, Writer};
use passage_core::{Direction, Packet, Phase, ProtocolVersion, versions};
use std::io;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Handshake {
    host: String,
}

impl Packet for Handshake {
    const NAME: &'static str = "Handshake";
    const PHASE: Phase = Phase::Handshake;
    const DIRECTION: Direction = Direction::Serverbound;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _v: ProtocolVersion) -> passage_core::wire::WireResult<Self> {
        Ok(Self {
            host: r.string("host", 255)?,
        })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _v: ProtocolVersion,
    ) -> passage_core::wire::WireResult<()> {
        w.string("host", &self.host)?;
        Ok(())
    }
}

#[tokio::test]
async fn client_talks_to_server_over_tcp() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let server_router = Arc::new(
        Router::<()>::builder()
            .on::<Handshake>(move |ctx, packet| {
                tx.send(packet.host).unwrap();
                ctx.handle.close()?;
                Ok(())
            })
            .unwrap()
            .build(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(
        Server::new(listener)
            .state(|_: &std::net::SocketAddr| ())
            .dispatch(server_router)
            .graceful_shutdown(shutdown.clone())
            .serve(),
    );

    let client_router = Arc::new(
        Router::<()>::builder()
            .on_open(|ctx| {
                ctx.handle.batch(|batch| {
                    batch.send(
                        ctx.version,
                        Handshake {
                            host: "mc.justchunks.net".to_owned(),
                        },
                    )?;
                    batch.close();
                    Ok(())
                })?;
                Ok::<(), DispatchError>(())
            })
            .build(),
    );

    let outcome = Client::new(addr)
        .state(|_: &std::net::SocketAddr| ())
        .dispatch(client_router)
        .connect()
        .await
        .unwrap();

    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(rx.recv().await.as_deref(), Some("mc.justchunks.net"));
    shutdown.cancel();
    server.await.unwrap();
}

#[tokio::test]
async fn preconnected_drives_a_duplex_pair() {
    let (a, b) = tokio::io::duplex(1024);
    let (tx, mut rx) = mpsc::unbounded_channel();

    let server_router = Arc::new(
        Router::<()>::builder()
            .on::<Handshake>(move |ctx, packet| {
                tx.send(packet.host).unwrap();
                ctx.handle.close()?;
                Ok(())
            })
            .unwrap()
            .build(),
    );
    let server = tokio::spawn(
        Client::new(Preconnected::new(b, "server"))
            .state(|_: &&str| ())
            .dispatch(server_router)
            .connect(),
    );

    let client_router = Arc::new(
        Router::<()>::builder()
            .on_open(|ctx| {
                ctx.handle.batch(|batch| {
                    batch.send(
                        ctx.version,
                        Handshake {
                            host: "duplex".to_owned(),
                        },
                    )?;
                    batch.close();
                    Ok(())
                })?;
                Ok::<(), DispatchError>(())
            })
            .build(),
    );

    let outcome = Client::new(Preconnected::new(a, "client"))
        .state(|_: &&str| ())
        .dispatch(client_router)
        .initial_version(versions::V26_2)
        .connect()
        .await
        .unwrap();

    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    assert_eq!(rx.recv().await.as_deref(), Some("duplex"));
    let _ = server.await.unwrap();
}

#[tokio::test]
async fn a_failed_dial_is_reported() {
    // Nothing listens on a port we bound and dropped.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let error = Client::new(addr)
        .state(|_: &std::net::SocketAddr| ())
        .connect()
        .await
        .expect_err("nothing is listening");
    assert!(matches!(
        error,
        passage_core::client::ClientError::Connect(ref err)
            if err.kind() == io::ErrorKind::ConnectionRefused
    ));
}
