use crate::protocol::{State, serve_handshake, serve_login, serve_ping, serve_status};
use crate::status::{ServerPlayers, ServerStatus, ServerVersion};
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use serde_json::value::RawValue;
use std::any::Any;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::debug;

pub async fn serve<S>(
    listener: TcpListener,
    keys: (RsaPrivateKey, RsaPublicKey),
    state: Arc<S>,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: Any,
{
    loop {
        let (mut socket, addr) = listener.accept().await?;
        let shake = serve_handshake(&mut socket).await?;

        match shake.state {
            State::Status => {
                let status = ServerStatus {
                    version: ServerVersion {
                        name: "JustChunks 2025".to_owned(),
                        protocol: shake.protocol,
                    },
                    players: Some(ServerPlayers {
                        online: 5,
                        max: 10,
                        sample: None,
                    }),
                    description: Some(RawValue::from_string(
                        r#"{"text":"PASSAGE IS RUNNING","color":"gold"}"#.to_string(),
                    )?),
                    favicon: None,
                    enforces_secure_chat: Some(true),
                };
                serve_status(&mut socket, &status).await?;
                serve_ping(&mut socket).await?;
            }
            State::Login => {
                serve_login(&mut socket, &keys).await?;
            }
            _ => {}
        }

        // flush connection and shutdown
        socket.shutdown().await?;
        debug!("connection closed: {}", addr);
    }
}
