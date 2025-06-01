#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod adapter;
pub mod authentication;
pub mod cipher_stream;
pub mod config;
pub mod connection;
mod metrics;
pub mod mojang;
pub mod rate_limiter;

use crate::adapter::resourcepack::ResourcepackSupplier;
#[cfg(feature = "grpc")]
use crate::adapter::resourcepack::grpc::GrpcResourcepackSupplier;
use crate::adapter::resourcepack::impackable::ImpackableResourcepackSupplier;
use crate::adapter::resourcepack::none::NoneResourcePackSupplier;
use crate::adapter::status::StatusSupplier;
#[cfg(feature = "grpc")]
use crate::adapter::status::grpc::GrpcStatusSupplier;
use crate::adapter::status::mongodb::MongodbStatusSupplier;
use crate::adapter::status::none::NoneStatusSupplier;
use crate::adapter::target_selection::TargetSelector;
#[cfg(feature = "agones")]
use crate::adapter::target_selection::agones::AgonesTargetSelector;
use crate::adapter::target_selection::fixed::FixedTargetSelector;
#[cfg(feature = "grpc")]
use crate::adapter::target_selection::grpc::GrpcTargetSelector;
use crate::adapter::target_selection::none::NoneTargetSelector;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::target_strategy::any::AnyTargetSelectorStrategy;
#[cfg(feature = "grpc")]
use crate::adapter::target_strategy::grpc::GrpcTargetSelectorStrategy;
use crate::adapter::target_strategy::none::NoneTargetSelectorStrategy;
use crate::adapter::target_strategy::player_fill::PlayerFillTargetSelectorStrategy;
use crate::config::Config;
use crate::connection::{Connection, Error};
use crate::metrics::{OPEN_CONNECTIONS, OpenConnectionsLabels, RequestsLabels};
use crate::mojang::Api;
use crate::mojang::Mojang;
use crate::rate_limiter::RateLimiter;
use adapter::resourcepack::fixed::FixedResourcePackSupplier;
use adapter::status::fixed::FixedStatusSupplier;
use http_body_util::Full;
use hyper::Response;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use metrics::REQUESTS;
use prometheus_client::encoding::text::encode;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::select;
use tokio::time::timeout;
use tracing::{Instrument, debug, error, info, warn};

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
    let (shutdown, _) = tokio::sync::broadcast::channel(1);
    let config = Arc::new(config);

    // start the metrics server in own thread
    let m_shutdown = shutdown.clone();
    let m_config = Arc::clone(&config);
    let metric_server = tokio::spawn(async move {
        if let Err(err) = start_metrics(m_config, m_shutdown.subscribe()).await {
            error!(cause = err.to_string(), "metrics server stopped with error");
        }
        m_shutdown.send(()).expect("failed to shut down");
    });

    // start the protocol server in own thread
    let p_shutdown = shutdown.clone();
    let p_config = Arc::clone(&config);
    let protocol_server = tokio::spawn(async move {
        if let Err(err) = start_protocol(p_config, p_shutdown.subscribe()).await {
            error!(
                cause = err.to_string(),
                "protocol server stopped with error"
            );
        }
        p_shutdown.send(()).expect("failed to shut down");
    });

    // wait for either a shutdown signal (from error) or a ctrl-c
    let mut shutdown_signal = shutdown.subscribe();
    select!(
        _ = shutdown_signal.recv() => {},
        _ = tokio::signal::ctrl_c() => {
            info!("received ctrl-c signal, shutting down");
            shutdown.send(()).expect("failed to shut down");
        },
    );

    // wait for threads to shut down
    _ = metric_server.await;
    _ = protocol_server.await;

    Ok(())
}

async fn start_metrics(
    config: Arc<Config>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!("starting metrics server");

    // start metrics server
    info!(
        addr = config.metrics_address.to_string(),
        "binding metrics socket address"
    );
    let listener = TcpListener::bind(&config.metrics_address).await?;

    // listen for connections
    loop {
        // accept the next incoming connection
        let (stream, addr) = select! {
            accepted = listener.accept() => accepted?,
            _ = shutdown.recv() => {
                break;
            },
        };
        debug!(addr = addr.to_string(), "received metrics connection");

        let io = TokioIo::new(stream);

        let connection_id = uuid::Uuid::new_v4();
        let connection_span = tracing::info_span!(
            "metrics",
            addr = addr.to_string(),
            id = connection_id.to_string(),
        );

        debug!(
            addr = addr.to_string(),
            "moving metrics connection to a new task"
        );
        tokio::task::spawn(
            async move {
                if let Err(err) = http1::Builder::new()
                    .timer(TokioTimer::new())
                    .serve_connection(
                        io,
                        service_fn(|_| async {
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
            }
            .instrument(connection_span),
        );
    }

    info!("metrics server stopped successfully");
    Ok(())
}

async fn start_protocol(
    config: Arc<Config>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    debug!("starting protocol server");

    // retrieve config params
    let timeout_duration = Duration::from_secs(config.timeout);
    let auth_secret = config.auth_secret.clone().map(String::into_bytes);
    let localization = Arc::new(config.localization.clone());

    // initialize status supplier
    debug!(
        adaper = config.status.adapter.as_str(),
        "initializing status supplier"
    );
    let status_supplier = match config.status.adapter.as_str() {
        "none" => Arc::new(NoneStatusSupplier) as Arc<dyn StatusSupplier>,
        #[cfg(feature = "grpc")]
        "grpc" => {
            let Some(grpc) = config.status.grpc.clone() else {
                return Err("grpc status adapter requires a configuration".into());
            };
            Arc::new(GrpcStatusSupplier::new(grpc).await?) as Arc<dyn StatusSupplier>
        }
        #[cfg(feature = "mongodb")]
        "mongodb" => {
            let Some(mongodb) = config.status.mongodb.clone() else {
                return Err("mongodb status adapter requires a configuration".into());
            };
            Arc::new(MongodbStatusSupplier::new(mongodb).await?) as Arc<dyn StatusSupplier>
        }
        "fixed" => {
            let Some(fixed) = config.status.fixed.clone() else {
                return Err("fixed status adapter requires a configuration".into());
            };
            // TODO maybe move protocol to fixed config?
            Arc::new(FixedStatusSupplier::new(fixed, config.protocol.clone()))
                as Arc<dyn StatusSupplier>
        }
        _ => return Err("unknown status supplier configured".into()),
    };

    // initialize target selector strategy
    debug!(
        adaper = config.target_strategy.adapter.as_str(),
        "initializing target selector strategy"
    );
    let target_strategy = match config.target_strategy.adapter.as_str() {
        "none" => Arc::new(NoneTargetSelectorStrategy) as Arc<dyn TargetSelectorStrategy>,
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
        "any" => Arc::new(AnyTargetSelectorStrategy) as Arc<dyn TargetSelectorStrategy>,
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // initialize target selector
    debug!(
        adaper = config.target_discovery.adapter.as_str(),
        "initializing target selector"
    );
    let target_selector = match config.target_discovery.adapter.as_str() {
        "none" => Arc::new(NoneTargetSelector::new(target_strategy)) as Arc<dyn TargetSelector>,
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
        _ => return Err("unknown target selector strategy configured".into()),
    };

    // initialize resourcepack supplier
    debug!(
        adaper = config.resourcepack.adapter.as_str(),
        "initializing resource pack supplier"
    );
    let resourcepack_supplier = match config.resourcepack.adapter.as_str() {
        "none" => Arc::new(NoneResourcePackSupplier) as Arc<dyn ResourcepackSupplier>,
        #[cfg(feature = "grpc")]
        "grpc" => {
            let Some(grpc) = config.resourcepack.grpc.clone() else {
                return Err("grpc resourcepack adapter requires a configuration".into());
            };
            Arc::new(GrpcResourcepackSupplier::new(grpc).await?) as Arc<dyn ResourcepackSupplier>
        }
        "impackable" => {
            let Some(impackable) = config.resourcepack.impackable.clone() else {
                return Err("impackable resourcepack adapter requires a configuration".into());
            };
            Arc::new(ImpackableResourcepackSupplier::new(
                impackable,
                Arc::clone(&localization),
            )?) as Arc<dyn ResourcepackSupplier>
        }
        "fixed" => {
            let Some(fixed) = config.resourcepack.fixed.clone() else {
                return Err("fixed resourcepack adapter requires a configuration".into());
            };
            Arc::new(FixedResourcePackSupplier::new(fixed)) as Arc<dyn ResourcepackSupplier>
        }
        _ => return Err("unknown target selector strategy configured".into()),
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

    loop {
        // accept the next incoming connection
        let (mut stream, addr) = select! {
            accepted = listener.accept() => accepted?,
            _ = shutdown.recv() => {
                break;
            },
        };
        debug!(addr = addr.to_string(), "received protocol connection");

        // check rate limiter
        if rate_limiter_enabled && !rate_limiter.enqueue(&addr.ip()) {
            info!(addr = addr.to_string(), "rate limited client");

            REQUESTS
                .get_or_create(&RequestsLabels { result: "rejected" })
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
        let mojang = Arc::clone(&mojang);
        let localization = Arc::clone(&localization);
        let auth_secret = auth_secret.clone();

        let connection_id = uuid::Uuid::new_v4();
        let connection_span = tracing::info_span!(
            "protocol",
            addr = addr.to_string(),
            id = connection_id.to_string(),
        );

        tokio::spawn(
            async move {
                info!("accepted new connection");
                OPEN_CONNECTIONS
                    .get_or_create(&OpenConnectionsLabels {})
                    .inc();

                // build connection wrapper for stream
                let mut con = Connection::new(
                    &mut stream,
                    Arc::clone(&status_supplier),
                    Arc::clone(&target_selector),
                    Arc::clone(&resourcepack_supplier),
                    Arc::clone(&mojang),
                    Arc::clone(&localization),
                    auth_secret,
                );

                // handle the client connection (ignore connection closed by the client)
                let timeout = timeout(timeout_duration, con.listen(addr)).await;
                let result = match timeout {
                    Ok(Err(Error::ConnectionClosed(_))) => "connection-closed",
                    Ok(Err(err)) => {
                        warn!(
                            cause = err.to_string(),
                            "failure communicating with a client"
                        );
                        err.as_label()
                    }
                    Ok(_) => "success",
                    Err(_) => "timeout",
                };

                // flush connection and shutdown
                if let Err(err) = stream.shutdown().await {
                    debug!(
                        cause = err.to_string(),
                        "failed to close a client connection"
                    );
                }

                REQUESTS.get_or_create(&RequestsLabels { result }).inc();

                OPEN_CONNECTIONS
                    .get_or_create(&OpenConnectionsLabels {})
                    .dec();

                debug!("closed connection with a client");
            }
            .instrument(connection_span),
        );
    }

    info!("protocol server stopped successfully");
    Ok(())
}
