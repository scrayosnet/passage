use crate::protocol::handle_client;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use std::any::Any;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

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
                    addr = &addr.to_string(),
                    "failure communicating with client"
                );
            }

            // flush connection and shutdown
            if let Err(e) = socket.shutdown().await {
                debug!(
                    addr = &addr.to_string(),
                    "failed to close client connection"
                );
            }

            debug!(addr = &addr.to_string(), "closed connection with client");
        });
    }
}
