use crate::connection::Connection;
use crate::status_supplier::StatusSupplier;
use crate::target_selector::TargetSelector;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::{debug, info};

pub async fn serve(
    listener: TcpListener,
    status_supplier: Arc<dyn StatusSupplier>,
    target_selector: Arc<dyn TargetSelector>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // accept the next incoming connection
        let (mut stream, addr) = listener.accept().await?;

        // clone values to be moved
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);

        tokio::spawn(async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                addr,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
            );

            // handle the client connection
            if let Err(e) = con.listen().await {
                info!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failure communicating with a client"
                );
            }

            // flush connection and shutdown
            if let Err(e) = stream.shutdown().await {
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
