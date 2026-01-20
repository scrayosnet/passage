#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod adapter;
pub mod authentication;
pub mod cipher_stream;
pub mod config;
pub mod connection;
mod metrics;
pub mod mojang;
pub mod proxy_protocol;
pub mod rate_limiter;

use crate::adapter::status::StatusSupplier;
#[cfg(feature = "grpc")]
use crate::adapter::status::grpc::GrpcStatusSupplier;
use crate::adapter::status::http::HttpStatusSupplier;
use crate::adapter::target_selection::TargetSelector;
#[cfg(feature = "agones")]
use crate::adapter::target_selection::agones::AgonesTargetSelector;
use crate::adapter::target_selection::fixed::FixedTargetSelector;
#[cfg(feature = "grpc")]
use crate::adapter::target_selection::grpc::GrpcTargetSelector;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::target_strategy::fixed::FixedTargetSelectorStrategy;
#[cfg(feature = "grpc")]
use crate::adapter::target_strategy::grpc::GrpcTargetSelectorStrategy;
use crate::adapter::target_strategy::player_fill::PlayerFillTargetSelectorStrategy;
use crate::config::Config;
use crate::connection::{Connection, Error};
use crate::mojang::Api;
use crate::mojang::Mojang;
use crate::rate_limiter::RateLimiter;
use adapter::status::fixed::FixedStatusSupplier;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::select;
use tokio::time::{Instant, timeout};
use tokio_util::task::TaskTracker;
use tracing::{Instrument, debug, info, warn};

/// Initializes the Minecraft tcp server and creates all necessary resources for the operation.
///
/// This binds the server socket and starts the TCP server to serve the login requests of the players. This also
/// configures the corresponding discoveries and adapters that are invoked on any login request for the socket. The
/// socket and protocol are made ready for the very first connection attempt.
///
/// # Errors
///
/// Will return an appropriate error if the socket cannot be bound to the supplied address, or the TCP server cannot be
/// properly initialized.
pub async fn start(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    debug!("starting protocol server");

    // retrieve config params
    let timeout_duration = Duration::from_secs(config.timeout);
    let auth_secret = config.auth_secret.clone().map(String::into_bytes);
    let localization = Arc::new(config.localization.clone());
    let proxy_protocol_enabled = config.proxy_protocol.enabled;

    // initialize status supplier
    debug!(
        adaper = config.status.adapter.as_str(),
        "initializing status supplier"
    );
    let status_supplier = match config.status.adapter.as_str() {
        #[cfg(feature = "grpc")]
        "grpc" => {
            let Some(grpc) = config.status.grpc.clone() else {
                return Err("grpc status adapter requires a configuration".into());
            };
            Arc::new(GrpcStatusSupplier::new(grpc).await?) as Arc<dyn StatusSupplier>
        }
        "http" => {
            let Some(http) = config.status.http.clone() else {
                return Err("http status adapter requires a configuration".into());
            };
            Arc::new(HttpStatusSupplier::new(http).await?) as Arc<dyn StatusSupplier>
        }
        "fixed" => {
            let Some(fixed) = config.status.fixed.clone() else {
                return Err("fixed status adapter requires a configuration".into());
            };
            Arc::new(FixedStatusSupplier::new(fixed)) as Arc<dyn StatusSupplier>
        }
        _ => return Err("unknown status supplier configured".into()),
    };

    // initialize target selector strategy
    debug!(
        adaper = config.target_strategy.adapter.as_str(),
        "initializing target selector strategy"
    );
    let target_strategy = match config.target_strategy.adapter.as_str() {
        #[cfg(feature = "grpc")]
        "grpc" => {
            let Some(grpc) = config.target_strategy.grpc.clone() else {
                return Err("grpc target strategy adapter requires a configuration".into());
            };
            Arc::new(GrpcTargetSelectorStrategy::new(grpc).await?)
                as Arc<dyn TargetSelectorStrategy>
        }
        "player_fill" => {
            let Some(player_fill) = config.target_strategy.player_fill.clone() else {
                return Err("player_fill target strategy adapter requires a configuration".into());
            };
            Arc::new(PlayerFillTargetSelectorStrategy::new(player_fill))
                as Arc<dyn TargetSelectorStrategy>
        }
        "fixed" => Arc::new(FixedTargetSelectorStrategy) as Arc<dyn TargetSelectorStrategy>,
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // initialize target selector
    debug!(
        adaper = config.target_discovery.adapter.as_str(),
        "initializing target selector"
    );
    let target_selector = match config.target_discovery.adapter.as_str() {
        #[cfg(feature = "grpc")]
        "grpc" => {
            let Some(grpc) = config.target_discovery.grpc.clone() else {
                return Err("grpc target discovery adapter requires a configuration".into());
            };
            Arc::new(GrpcTargetSelector::new(target_strategy, grpc).await?)
                as Arc<dyn TargetSelector>
        }
        #[cfg(feature = "agones")]
        "agones" => {
            let Some(agones) = config.target_discovery.agones.clone() else {
                return Err("agones target discovery adapter requires a configuration".into());
            };
            Arc::new(AgonesTargetSelector::new(target_strategy, agones).await?)
                as Arc<dyn TargetSelector>
        }
        "fixed" => {
            let Some(fixed) = config.target_discovery.fixed.clone() else {
                return Err("fixed target selector adapter requires a configuration".into());
            };
            Arc::new(FixedTargetSelector::new(target_strategy, fixed)) as Arc<dyn TargetSelector>
        }
        _ => return Err("unknown target selector discovery configured".into()),
    };

    // initialize mojang client
    debug!("initializing mojang client");
    let mojang = Arc::new(Api::default()) as Arc<dyn Mojang>;

    // bind the socket address on all interfaces
    info!(addr = config.address.to_string(), "binding socket address");
    let listener = TcpListener::bind(&config.address).await?;

    // initialize rate limiting and timeout
    debug!(
        enabled = config.rate_limiter.enabled,
        "initializing rate limiter"
    );
    let rate_limiter_enabled = config.rate_limiter.enabled;
    let mut rate_limiter = RateLimiter::new(
        Duration::from_secs(config.rate_limiter.duration),
        config.rate_limiter.size,
    );

    // initialize connection tracker
    let tracker = TaskTracker::new();

    loop {
        // accept the next incoming connection
        let (mut stream, addr) = select! {
            accepted = listener.accept() => accepted?,
            _ = tokio::signal::ctrl_c() => {
                info!("received ctrl-c signal, shutting down");
                break;
            },
        };
        let connection_start = Instant::now();

        // extract real client address from PROXY protocol header if enabled
        let client_addr = if proxy_protocol_enabled {
            match proxy_protocol::read_proxy_header(&mut stream).await {
                Ok(real_addr) => {
                    debug!(
                        proxy_addr = addr.to_string(),
                        real_addr = real_addr.to_string(),
                        "extracted real client address from PROXY protocol"
                    );
                    real_addr
                }
                Err(e) => {
                    warn!(
                        cause = e.to_string(),
                        addr = addr.to_string(),
                        "failed to read PROXY protocol header, rejecting connection"
                    );
                    metrics::request_duration::record(connection_start, "proxy-protocol-error");
                    if let Err(e) = stream.shutdown().await {
                        debug!(
                            cause = e.to_string(),
                            addr = addr.to_string(),
                            "failed to close a client connection"
                        );
                    }
                    continue;
                }
            }
        } else {
            addr
        };

        debug!(addr = client_addr.to_string(), "received protocol connection");

        // check rate limiter (use real client address)
        if rate_limiter_enabled && !rate_limiter.enqueue(client_addr.ip()) {
            info!(addr = client_addr.to_string(), "rate limited client");
            metrics::request_duration::record(connection_start, "rejected");

            if let Err(e) = stream.shutdown().await {
                debug!(
                    cause = e.to_string(),
                    addr = &client_addr.to_string(),
                    "failed to close a client connection"
                );
            }
            continue;
        }

        // clone values to be moved
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);
        let mojang = Arc::clone(&mojang);
        let localization = Arc::clone(&localization);
        let auth_secret = auth_secret.clone();

        let connection_id = uuid::Uuid::new_v4();
        let connection_span = tracing::info_span!(
            "protocol",
            addr = client_addr.to_string(),
            id = connection_id.to_string(),
        );

        tracker.spawn(
            async move {
                info!("accepted new connection");
                metrics::open_connections::inc();

                // build connection wrapper for stream
                let mut con = Connection::new(
                    &mut stream,
                    Arc::clone(&status_supplier),
                    Arc::clone(&target_selector),
                    Arc::clone(&mojang),
                    Arc::clone(&localization),
                    auth_secret,
                );

                // handle the client connection (ignore connection closed by the client)
                let timeout = timeout(timeout_duration, con.listen(client_addr)).await;
                let connection_result = match timeout {
                    Ok(Err(Error::ConnectionClosed(_))) => "connection-closed",
                    Ok(Err(err)) => {
                        warn!(cause = err.to_string(), "failed to handle connection");
                        err.as_label()
                    }
                    Ok(_) => "success",
                    Err(_) => "timeout",
                };

                // flush connection and shutdown
                if let Err(err) = stream.shutdown().await {
                    warn!(cause = err.to_string(), "failed to shutdown connection");
                }
                debug!("closed connection");

                // update metrics
                metrics::request_duration::record(connection_start, connection_result);
                metrics::open_connections::dec();
            }
            .instrument(connection_span),
        );
    }

    // wait for all connections to finish
    tracker.close();
    tracker.wait().await;

    info!("protocol server stopped successfully");
    Ok(())
}
