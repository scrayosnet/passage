use crate::connection::Connection;
use crate::resource_pack_supplier::ResourcePackSupplier;
use crate::status_supplier::StatusSupplier;
use crate::target_selector::TargetSelector;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tracing::{info, warn};

pub async fn serve(
    listener: TcpListener,
    status_supplier: Arc<dyn StatusSupplier>,
    target_selector: Arc<dyn TargetSelector>,
    resource_pack_supplier: Arc<dyn ResourcePackSupplier>,
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
        let resource_pack_supplier = Arc::clone(&resource_pack_supplier);

        tokio::spawn(async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                addr,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
                Arc::clone(&resource_pack_supplier),
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
        });
    }
}
