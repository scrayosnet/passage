use crate::core::{StatusSupplier, TargetSelector};
use crate::protocol::handle_client;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::{debug, info};

pub async fn serve<SS, TS>(
    listener: TcpListener,
    keys: (RsaPrivateKey, RsaPublicKey),
    status_supplier: SS,
    target_selector: TS,
) -> Result<(), Box<dyn std::error::Error>>
where
    SS: StatusSupplier + Send + Sync + 'static,
    TS: TargetSelector + Send + Sync + 'static,
{
    let keys = Arc::new(keys);
    let status_supplier = Arc::new(status_supplier);
    let target_selector = Arc::new(target_selector);

    loop {
        // accept the next incoming connection
        let (mut socket, addr) = listener.accept().await?;

        let keys = Arc::clone(&keys);
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);

        tokio::spawn(async move {
            // handle the client connection
            if let Err(e) = handle_client(
                &mut socket,
                &addr,
                keys.as_ref(),
                status_supplier,
                target_selector,
            )
            .await
            {
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
