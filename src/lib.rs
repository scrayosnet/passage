#![deny(clippy::all)]
#![forbid(unsafe_code)]

mod adapter;
pub mod authentication;
pub mod config;
mod connection;
mod protocol;
mod rate_limiter;
mod server;
mod status;

use crate::adapter::target_selection::Target;
use crate::adapter::target_selection::fixed::FixedTargetSelector;
use crate::adapter::target_strategy::any::AnyTargetSelectorStrategy;
use crate::config::Config;
use crate::status::{ServerPlayers, ServerStatus, ServerVersion};
use adapter::resourcepack::fixed::FixedResourcePackSupplier;
use adapter::status::simple::SimpleStatusSupplier;
use serde_json::value::RawValue;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

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
    // bind the socket address on all interfaces
    info!(addr = config.address.to_string(), "binding socket address");
    let listener = TcpListener::bind(&config.address).await?;

    // initialize services
    let status_supplier = SimpleStatusSupplier::from_status(ServerStatus {
        version: ServerVersion {
            name: "JustChunks 2025".to_owned(),
            protocol: 0,
        },
        players: Some(ServerPlayers {
            online: 5,
            max: 10,
            sample: None,
        }),
        description: Some(RawValue::from_string(
            r#"{"text":"PASSAGE IS RUNNING","color":"gold"}"#.to_string(),
        )?),
        favicon: None,
        enforces_secure_chat: Some(true),
    });
    let target = Target {
        identifier: "test".to_string(),
        address: SocketAddr::from_str("116.202.130.184:26426")?,
        meta: HashMap::<String, String>::default(),
    };
    let target_strategy = AnyTargetSelectorStrategy::new();
    let target_selector =
        FixedTargetSelector::from_targets(Arc::new(target_strategy), vec![target]);
    let resourcepack_supplier = FixedResourcePackSupplier;

    // serve the router service on the bound socket address
    server::serve(
        config,
        listener,
        Arc::new(status_supplier),
        Arc::new(target_selector),
        Arc::new(resourcepack_supplier),
    )
    .await?;

    info!("protocol server stopped successfully");
    Ok(())
}
