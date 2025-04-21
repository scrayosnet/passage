#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod adapter;
pub mod authentication;
pub mod cipher_stream;
pub mod config;
pub mod connection;
mod metrics;
pub mod rate_limiter;

use crate::adapter::resourcepack::none::NoneResourcePackSupplier;
use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::none::NoneStatusSupplier;
use crate::adapter::status::StatusSupplier;
use crate::adapter::target_selection::fixed::FixedTargetSelector;
use crate::adapter::target_selection::none::NoneTargetSelector;
use crate::adapter::target_selection::{Target, TargetSelector};
use crate::adapter::target_strategy::any::AnyTargetSelectorStrategy;
use crate::adapter::target_strategy::none::NoneTargetSelectorStrategy;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::Config;
use crate::connection::Connection;
use crate::metrics::{RateLimitedLabels, RequestLabels};
use crate::rate_limiter::RateLimiter;
use adapter::resourcepack::fixed::FixedResourcePackSupplier;
use adapter::status::fixed::FixedStatusSupplier;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Response;
use hyper_util::rt::{TokioIo, TokioTimer};
use prometheus_client::encoding::text::encode;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tracing::{debug, info, warn};

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
    // TODO shutdown signal!
    tokio::try_join!(start_mc(config.clone()), start_metrics(config)).map(|_| ())
}

async fn start_metrics(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // start metrics server
    info!(
        addr = config.metrics_address.to_string(),
        "binding metrics socket address"
    );
    let listener = TcpListener::bind(&config.metrics_address).await?;

    // listen for connections
    loop {
        let (stream, addr) = listener.accept().await?;
        let io = TokioIo::new(stream);
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .timer(TokioTimer::new())
                .serve_connection(
                    io,
                    service_fn(|_| async {
                        // TODO: handle errors
                        let mut buf = String::new();
                        encode(&mut buf, &metrics::REGISTRY).expect("failed to encode metrics");

                        let response = Response::builder()
                            .header(
                                hyper::header::CONTENT_TYPE,
                                "application/openmetrics-text; version=1.0.0; charset=utf-8",
                            )
                            .body(Full::<Bytes>::from(buf))
                            .expect("failed to build response");

                        Ok::<Response<Full<Bytes>>, Infallible>(response)
                    }),
                )
                .await
            {
                warn!(
                    cause = err.to_string(),
                    addr = &addr.to_string(),
                    "failure communicating with a client"
                );
            }
        });
    }
}

async fn start_mc(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // retrieve config params
    let timeout_duration = Duration::from_secs(config.timeout);
    let auth_secret = config.auth_secret.map(String::into_bytes);

    // initialize status supplier
    let status_supplier = match config.status.adapter.as_str() {
        "none" => Arc::new(NoneStatusSupplier) as Arc<dyn StatusSupplier>,
        "fixed" => {
            let Some(fixed) = config.status.fixed.clone() else {
                return Err("fixed status adapter requires a configuration".into());
            };
            Arc::new(FixedStatusSupplier::new(config.protocol.clone(), fixed))
        }
        _ => return Err("unknown status supplier configured".into()),
    };

    // initialize target selector strategy
    let target_strategy = match config.target_strategy.adapter.as_str() {
        "none" => Arc::new(NoneTargetSelectorStrategy) as Arc<dyn TargetSelectorStrategy>,
        "any" => Arc::new(AnyTargetSelectorStrategy),
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // initialize target selector
    let target_selector = match config.target.adapter.as_str() {
        "none" => Arc::new(NoneTargetSelector) as Arc<dyn TargetSelector>,
        "fixed" => {
            let Some(fixed) = config.target.fixed.clone() else {
                return Err("fixed target selector adapter requires a configuration".into());
            };
            let target = Target {
                identifier: fixed.identifier,
                address: fixed.address,
                meta: HashMap::<String, String>::default(),
            };
            Arc::new(FixedTargetSelector::new(target_strategy, vec![target]))
        }
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // initialize resourcepack supplier
    let resourcepack_supplier = match config.resourcepack.adapter.as_str() {
        "none" => Arc::new(NoneResourcePackSupplier) as Arc<dyn ResourcepackSupplier>,
        "fixed" => {
            let Some(fixed) = config.resourcepack.fixed.clone() else {
                return Err("fixed resourcepack adapter requires a configuration".into());
            };
            Arc::new(FixedResourcePackSupplier::new(fixed.packs))
        }
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // bind the socket address on all interfaces
    info!(addr = config.address.to_string(), "binding socket address");
    let listener = TcpListener::bind(&config.address).await?;

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
                break;
            },
        };

        metrics::REQUESTS.get_or_create(&RequestLabels {}).inc();

        // check rate limiter
        if rate_limiter_enabled && !rate_limiter.enqueue(&addr.ip()) {
            debug!(addr = addr.to_string(), "rate limited client");
            metrics::RATE_LIMITED
                .get_or_create(&RateLimitedLabels {})
                .inc();

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
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);
        let resourcepack_supplier = Arc::clone(&resourcepack_supplier);
        let auth_secret = auth_secret.clone();

        tokio::spawn(timeout(timeout_duration, async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
                Arc::clone(&resourcepack_supplier),
                auth_secret,
            );

            // handle the client connection
            if let Err(err) = con.listen(addr).await {
                warn!(
                    cause = err.to_string(),
                    addr = &addr.to_string(),
                    "failure communicating with a client"
                );
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

    info!("protocol server stopped successfully");
    Ok(())
}
