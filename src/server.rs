use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::StatusSupplier;
use crate::adapter::target_selection::TargetSelector;
use crate::config::Config;
use crate::connection::Connection;
use crate::rate_limiter::RateLimiter;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tracing::{debug, warn};

pub async fn serve(
    config: Config,
    listener: TcpListener,
    status_supplier: Arc<dyn StatusSupplier>,
    target_selector: Arc<dyn TargetSelector>,
    resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
) -> Result<(), Box<dyn std::error::Error>> {
    // retrieve config params
    let timeout_duration = Duration::from_secs(config.timeout);
    let auth_secret = config.auth_secret.map(|str| str.into_bytes());

    // setup rate limiting and timeout
    let rate_limiter_enabled = config.rate_limiter.enabled;
    let mut rate_limiter = RateLimiter::new(
        Duration::from_secs(config.rate_limiter.duration),
        config.rate_limiter.size,
    );

    loop {
        // accept the next incoming connection
        let (mut stream, addr) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = tokio::signal::ctrl_c() => {
                return Ok(());
            },
        };

        // check rate limiter
        if rate_limiter_enabled && !rate_limiter.enqueue(&addr.ip()) {
            debug!(addr = addr.to_string(), "rate limited client");
            if let Err(e) = stream.shutdown().await {
                debug!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failed to close a client connection"
                );
            }
            continue;
        }

        // clone values to be moved
        let timeout_duration = timeout_duration.clone();
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);
        let resourcepack_supplier = Arc::clone(&resourcepack_supplier);
        let auth_secret = auth_secret.clone();

        tokio::spawn(timeout(timeout_duration, async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                addr,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
                Arc::clone(&resourcepack_supplier),
                auth_secret,
            );

            // handle the client connection
            if let Err(err) = con.listen().await {
                if !err.is_connection_closed() {
                    warn!(
                        cause = err.to_string(),
                        addr = &addr.to_string(),
                        "failure communicating with a client"
                    );
                }
            }

            // flush connection and shutdown
            if let Err(err) = stream.shutdown().await {
                debug!(
                    cause = err.to_string(),
                    addr = &addr.to_string(),
                    "failed to close a client connection"
                );
            }

            debug!(addr = &addr.to_string(), "closed connection with a client");
        }));
    }
}
