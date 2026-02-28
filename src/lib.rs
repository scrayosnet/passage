#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod adapter;
pub mod config;

use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryAdapter;
use crate::adapter::filter::DynFilterAdapters;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::adapter::strategy::DynStrategyAdapter;
use crate::config::Config;
use passage_adapters::Adapters;
use passage_protocol::protocol::config::Config as ListenerConfig;
use passage_protocol::protocol::config::ProxyProtocol;
use passage_protocol::protocol::listener::Listener;
use passage_protocol::rate_limiter::RateLimiter;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

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
    // initialize the adapters
    debug!("building adapters");
    let status = DynStatusAdapter::from_config(config.adapters.status).await?;
    let discovery = DynDiscoveryAdapter::from_config(config.adapters.discovery).await?;
    let filters = DynFilterAdapters::from_config(config.adapters.filter).await?;
    let strategy = DynStrategyAdapter::from_config(config.adapters.strategy).await?;
    let authentication =
        DynAuthenticationAdapter::from_config(config.adapters.authentication).await?;
    let localization = DynLocalizationAdapter::from_config(config.adapters.localization).await?;
    let adapters = Adapters::new(
        status,
        discovery,
        filters,
        strategy,
        authentication,
        localization,
    );
    info!(adapters = %adapters,"build adapters");

    // initialize the rate limiter
    let rate_limiter = config.rate_limiter.map(|config| {
        RateLimiter::<IpAddr>::new(Duration::from_secs(config.duration), config.limit)
    });

    // build stop signal
    let stop_token = CancellationToken::new();
    let stop_token_signal = stop_token.clone();
    tokio::spawn(async move {
        // the thread will stop if either the stop signal is received of the application stops
        tokio::select! {
            _ = tokio::signal::ctrl_c() => stop_token_signal.cancel(),
            _ = stop_token_signal.cancelled() => { },
        }
    });

    // build and start the protocol
    debug!("building protocol");
    let listener_config = ListenerConfig {
        auth_secret: config.auth_secret.clone(),
        max_packet_length: config.max_packet_length as i32,
        auth_cookie_expiry: config.auth_cookie_expiry,
        proxy_protocol: config.proxy_protocol.map(|c| ProxyProtocol {
            allow_v1: c.allow_v1,
            allow_v2: c.allow_v2,
        }),
        connection_timeout: config.timeout,
    };
    let mut listener = Listener::new(Arc::new(adapters), rate_limiter, listener_config);

    debug!("starting protocol");
    listener.listen(config.address, stop_token.clone()).await?;
    stop_token.cancel();
    Ok(())
}
