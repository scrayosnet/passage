use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::StatusSupplier;
use crate::adapter::target_selection::TargetSelector;
use crate::connection::Connection;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tracing::{info, warn};

pub async fn serve(
    listener: TcpListener,
    status_supplier: Arc<dyn StatusSupplier>,
    target_selector: Arc<dyn TargetSelector>,
    resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        // accept the next incoming connection
        let (mut stream, addr) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = tokio::signal::ctrl_c() => {
                info!("received connection ctrl_c signal");
                return Ok(());
            },
        };

        // clone values to be moved
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);
        let resourcepack_supplier = Arc::clone(&resourcepack_supplier);

        tokio::spawn(timeout(Duration::from_secs(5), async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                addr,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
                Arc::clone(&resourcepack_supplier),
            );

            // handle the client connection
            if let Err(e) = con.listen().await {
                warn!(
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

            info!(addr = &addr.to_string(), "closed connection with a client");
        }));
    }
}
