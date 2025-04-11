#![feature(let_chains)]
#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod authentication;
pub mod config;
mod connection;
mod protocol;
mod resource_pack_supplier;
mod server;
mod status;
mod status_supplier;
mod target_selector;
mod target_selector_strategy;

use crate::config::Config;
use crate::resource_pack_supplier::none::NoneResourcePackSupplier;
use crate::resource_pack_supplier::test::TestResourcePackSupplier;
use crate::status::{ServerPlayers, ServerStatus, ServerVersion};
use crate::status_supplier::simple::SimpleStatusSupplier;
use crate::target_selector::first::SimpleTargetSelector;
use serde_json::value::RawValue;
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
pub async fn start(state: Config) -> Result<(), Box<dyn std::error::Error>> {
    // bind the socket address on all interfaces
    let addr = state.address;
    info!(addr = addr.to_string(), "binding socket address");
    let listener = TcpListener::bind(addr).await?;
    info!(addr = addr.to_string(), "successfully bound server socket");

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
    let target_selector =
        SimpleTargetSelector::from_target(SocketAddr::from_str("116.202.130.184:26426")?);

    let resource_pack_supplier = TestResourcePackSupplier;

    // serve the router service on the bound socket address
    server::serve(
        listener,
        Arc::new(status_supplier),
        Arc::new(target_selector),
        Arc::new(resource_pack_supplier),
    )
    .await?;
    info!("protocol server stopped successfully");

    // exit with success
    Ok(())
}
