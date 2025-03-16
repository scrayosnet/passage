#![deny(clippy::all)]
#![forbid(unsafe_code)]

pub mod authentication;
pub mod config;
mod core;
mod protocol;
mod server;
mod status;

use crate::config::AppState;
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
pub async fn start(state: Arc<AppState>) -> Result<(), Box<dyn std::error::Error>> {
    // generate a new key pair
    info!(
        bits = state.key_length,
        "generating a new cryptographic keypair"
    );
    let keys = authentication::generate_keypair()?;

    // bind the socket address on all interfaces
    let addr = state.address;
    info!(addr = addr.to_string(), "binding socket address");
    let listener = TcpListener::bind(addr).await?;
    info!(addr = addr.to_string(), "successfully bound server socket");

    // serve the router service on the bound socket address
    server::serve(listener, keys, state).await?;
    info!("protocol server stopped successfully");

    // exit with success
    Ok(())
}
