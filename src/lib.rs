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
use passage_protocol::listener::{Listener, ParseConfig};
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

    info!(
        status = %status,
        discovery = %discovery,
        filters = %filters,
        strategy = %strategy,
        authentication = %authentication,
        localization = %localization,
        "build adapters",
    );

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

    // retrieve config params
    let timeout_duration = Duration::from_secs(config.timeout);
    let auth_secret = config.auth_secret.clone().map(String::into_bytes);

    // build and start the listener
    debug!("building listener");
    let mut listener = Listener::new(
        Arc::new(status),
        Arc::new(discovery),
        Arc::new(filters),
        Arc::new(strategy),
        Arc::new(authentication),
        Arc::new(localization),
    )
    .with_rate_limiter(rate_limiter)
    .with_auth_secret(auth_secret)
    .with_connection_timeout(timeout_duration)
    .with_proxy_protocol(config.proxy_protocol.map(|config| ParseConfig {
        include_tlvs: false,
        allow_v1: config.allow_v1,
        allow_v2: config.allow_v2,
    }));

    debug!("starting listener");
    listener.listen(config.address, stop_token.clone()).await?;
    stop_token.cancel();
    Ok(())
}
