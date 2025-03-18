use crate::protocol::handle_client;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use std::any::Any;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::{debug, info};

pub async fn serve<S>(
    listener: TcpListener,
    keys: (RsaPrivateKey, RsaPublicKey),
    state: Arc<S>,
) -> Result<(), Box<dyn std::error::Error>>
where
    S: Any,
{
    let keys = Arc::new(keys);

    loop {
        // accept the next incoming connection
        let (mut socket, addr) = listener.accept().await?;
        let keys = Arc::clone(&keys);

        tokio::spawn(async move {
            // handle the client connection
            if let Err(e) = handle_client(&mut socket, &keys).await {
                info!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failure communicating with a client"
                );
            }

            // flush connection and shutdown
            if let Err(e) = socket.shutdown().await {
                info!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failed to close a client connection"
                );
            }

            debug!(addr = &addr.to_string(), "closed connection with a client");
        });
    }
}
