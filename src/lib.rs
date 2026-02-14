#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod config;
pub mod discovery_adapter;
pub mod status_adapter;
pub mod strategy_adapter;

use crate::config::Config;
use crate::discovery_adapter::DynDiscoveryAdapter;
use crate::status_adapter::DynStatusAdapter;
use crate::strategy_adapter::DynStrategyAdapter;
use passage_protocol::listener::Listener;
use passage_protocol::localization::Localization;
use passage_protocol::mojang::Api;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::debug;

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
    let status_adapter = Arc::new(DynStatusAdapter::from_config(&config.status).await?);
    let discovery_adapter =
        Arc::new(DynDiscoveryAdapter::from_config(&config.target_discovery).await?);
    let strategy_adapter =
        Arc::new(DynStrategyAdapter::from_config(&config.target_strategy).await?);
    let mojang = Arc::new(Api::default());
    let localization = Arc::new(Localization {
        default_locale: config.localization.default_locale,
        messages: config.localization.messages,
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
        status_adapter,
        discovery_adapter,
        strategy_adapter,
        mojang,
        localization,
    )
    .with_auth_secret_opt(auth_secret)
    .with_connection_timeout(timeout_duration)
    .with_proxy_protocol(config.proxy_protocol.enabled);

    debug!("starting listener");
    listener.listen(config.address, stop_token.clone()).await?;
    stop_token.cancel();
    Ok(())
}
