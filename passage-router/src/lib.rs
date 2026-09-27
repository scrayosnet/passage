//! An opinionated implementation of the Minecraft: Java Edition protocol, tailored to routing players
//! with the [transfer packet](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer_(configuration))
//! introduced in Minecraft 1.20.5.
//!
//! # Overview
//!
//! [`start`] binds the listener and accepts TCP connections, handing each one to a connection that
//! drives the protocol state machine `Handshake → Status | Login → Configuration → Transfer`. The
//! hostname from the handshake is matched against the configured [`router::DynRoutes`]; the matching
//! [`adapter::Route`] supplies the adapters that answer status pings, authenticate the
//! player, localize disconnect messages and discover the transfer target. After the transfer packet
//! has been sent, the connection is dropped -- no state about the player is retained.
//!
//! # Starting up
//!
//! A binary wires the three steps together and nothing else:
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let config = passage_router::config::Config::read()?;
//! let _telemetry = passage_router::init_tracing(&config)?;
//! passage_router::start(config).await
//! # }
//! ```

#![forbid(unsafe_code)]
// Each module keeps its centrepiece in a file of its own name (`adapter/adapter.rs`) so that
// `mod.rs` stays the module's documentation and re-export list, as in `passage-core`.
#![allow(clippy::module_inception)]

pub mod adapter;
pub mod config;
pub mod cookie;
pub mod crypto;
pub mod error;
pub mod layer;
pub mod metrics;
pub mod router;
mod telemetry;

use crate::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::config::Config;
use crate::layer::{MetricsLayer, ProxyProtocolLayer, RateLimiterLayer};
use crate::router::{DynRoute, DynRoutes, State};
pub use error::*;
use passage_core::server::Server;
use passage_core::wire::{Bytes, Options as WireOptions};
use regex::Regex;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument};

pub use telemetry::{Telemetry, init_tracing};

/// Runs Passage until it is asked to stop.
///
/// Binds the listener, turns the configured routes into adapters, and serves connections until
/// `SIGINT` or `SIGTERM` arrives -- at which point every live connection is told why it is going
/// away and the server drains.
///
/// # Errors
///
/// Returns an error if the address cannot be bound, a hostname pattern is not a valid regex, an
/// adapter cannot establish its connection, or the router is misconfigured. All of these happen
/// during startup: once the server is serving, a failure is a single connection's and not the
/// process's.
pub async fn start(mut config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // Build the adapters into routes first: an adapter that cannot connect should fail the start
    // rather than the first player unlucky enough to hit it.
    let routes = build_routes(&mut config).await?;

    // Build the server listener. This is the last thing that can fail, so that a process which is
    // listening is one that is ready to answer.
    let listener = TcpListener::bind(&config.address).await?;
    info!(
        address = %config.address,
        routes = routes.len(),
        timeout = config.timeout,
        max_packet_length = config.max_packet_length,
        "listening",
    );

    // Builds the server layers. Each of them admits everything when its config is absent.
    let proxy_layer = ProxyProtocolLayer::new(config.proxy_protocol.take());
    let limiter_layer = RateLimiterLayer::new(config.rate_limiter.take());

    // Observe the host the process runs on, if that was asked for.
    let observer = config
        .system_observer_interval
        .map(|seconds| metrics::system::Observer::new(Duration::from_secs(seconds)));

    // Build the server with the listener, router, layers, and state.
    let token = CancellationToken::new();
    let auth_secret = config
        .auth_secret
        .take()
        .map(|secret| Bytes::copy_from_slice(secret.as_bytes()));
    let auth_cookie_expiry = config.auth_cookie_expiry;
    let server = Server::new(listener)
        .layer(proxy_layer)
        .layer(limiter_layer)
        .layer(MetricsLayer)
        .dispatch(Arc::new(router::router()?))
        .wire_options(WireOptions {
            max_frame_len: config.max_packet_length,
            ..WireOptions::default()
        })
        .max_lifetime(Some(Duration::from_secs(config.timeout)))
        .state(move |addr| {
            State::new(
                routes.clone(),
                *addr,
                auth_secret.clone(),
                auth_cookie_expiry,
            )
        })
        .graceful_shutdown(token.child_token());

    // Run the server until it is asked to stop. Cancelling the token is what tells every live
    // connection to say goodbye, so the server is awaited afterwards rather than dropped.
    let mut serve_handle = tokio::spawn(server.serve());
    tokio::select! {
        result = &mut serve_handle => result?,
        reason = terminate() => {
            info!(reason, "shutting down");
            token.cancel();
            serve_handle.await?;
        },
    }

    if let Some(observer) = observer {
        observer.shutdown().await;
    }
    info!("stopped");
    Ok(())
}

/// Turns the configured routes into the adapters that answer them.
#[instrument(skip_all, fields(routes = config.routes.len()))]
async fn build_routes(config: &mut Config) -> Result<DynRoutes, Box<dyn std::error::Error>> {
    let mut routes: Vec<Arc<DynRoute>> = Vec::with_capacity(config.routes.len());
    for route in std::mem::take(&mut config.routes) {
        debug!(hostname = %route.hostname, "building route");
        routes.push(Arc::new(Route {
            hostname: Regex::new(&route.hostname)?,
            status: DynStatusAdapter::from_config(route.status).await?,
            discovery: DynDiscoveryActionAdapter::from_config(route.discovery).await?,
            authentication: DynAuthenticationAdapter::from_config(route.authentication).await?,
            localization: DynLocalizationAdapter::from_config(route.localization).await?,
        }));
    }
    for route in &routes {
        info!(
            hostname = %route.hostname,
            status = %route.status,
            authentication = %route.authentication,
            localization = %route.localization,
            discovery = route.discovery.iter().map(ToString::to_string).collect::<Vec<_>>().join(" -> "),
            "built route",
        );
    }
    Ok(routes.into())
}

/// Resolves once the process is asked to stop, naming the signal that asked.
///
/// `SIGTERM` is what an orchestrator sends, and `SIGINT` is what a terminal sends. Passage answers
/// both the same way; only the log line differs.
async fn terminate() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(terminate) => terminate,
            Err(err) => {
                // Without the handler there is still `ctrl_c`, so this is a degraded shutdown and
                // not a failed start.
                tracing::warn!(cause = %err, "could not listen for SIGTERM");
                let _ = tokio::signal::ctrl_c().await;
                return "SIGINT";
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            _ = terminate.recv() => "SIGTERM",
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        "SIGINT"
    }
}
