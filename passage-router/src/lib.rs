//! An opinionated implementation of the Minecraft: Java Edition protocol, tailored to routing players
//! with the [transfer packet](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer_(configuration))
//! introduced in Minecraft 1.20.5.
//!
//! # Overview
//!
//! The [`listener`] accepts TCP connections and hands each one to a [`connection::Connection`], which
//! drives the protocol state machine `Handshake → Status | Login → Configuration → Transfer`. The
//! hostname from the handshake is matched against the configured [`adapter::Routes`]; the matching
//! [`adapter::Route`] supplies the adapters that answer status pings, authenticate the player, localize
//! disconnect messages and discover the transfer target. After the transfer packet has been sent, the
//! connection is dropped -- no state about the player is retained.

pub mod adapter;
pub mod config;
pub mod cookie;
pub mod crypto;
pub mod error;
pub mod metrics;
pub mod router;
pub mod layer;

use std::sync::Arc;
use regex::Regex;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
pub use error::*;
use passage_core::router::Router;
use passage_core::server::Server;
use passage_core::wire::Bytes;
use crate::adapter::adapter::Route;
use crate::adapter::authentication::DynAuthenticationAdapter;
use crate::adapter::discovery::DynDiscoveryActionAdapter;
use crate::adapter::localization::DynLocalizationAdapter;
use crate::adapter::status::DynStatusAdapter;
use crate::config::Config;
use crate::layer::{RateLimiterLayer, ProxyProtocolLayer};
use crate::router::{DynRoute, DynRoutes, State};

pub fn init_tracing(config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    // TODO implement me!
    Ok(())
}

pub async fn start(mut config: Config) -> Result<(), Box<dyn std::error::Error>> {
    // Build the server listener.
    let listener = TcpListener::bind("").await?;

    // Builds the server layers.
    let limiter_layer = RateLimiterLayer::new(config.rate_limiter.take());
    let proxy_layer = ProxyProtocolLayer::new(config.proxy_protocol.take());

    // Build the adapters into routes.
    let mut routes: Vec<Arc<DynRoute>> = vec![];
    for route in config.routes {
        routes.push(Arc::new(Route {
            hostname: Regex::new(&route.hostname)?,
            status: DynStatusAdapter::from_config(route.status).await?,
            discovery: DynDiscoveryActionAdapter::from_config(route.discovery).await?,
            authentication: DynAuthenticationAdapter::from_config(route.authentication).await?,
            localization: DynLocalizationAdapter::from_config(route.localization).await?,
        }));
    }
    let routes: DynRoutes = routes.into();

    // Build the server router.
    let router = Router::builder()
        .on_open(router::on_open)
        .on(router::on_handshake_intention_packet)?
        .on(router::on_status_request_packet)?
        .on(router::on_status_ping_request_packet)?
        .on(router::on_login_login_start)?
        .on(router::on_login_encryption_response)?
        .on(router::on_login_login_acknowledged)?
        .on(router::on_login_cookie_response)?
        .on(router::on_configuration_client_information)?
        .on(router::on_configuration_keep_alive)?
        .on(router::on_configuration_cookie_response)?
        .on(router::on_configuration_custom_payload)?
        .on(router::on_configuration_resource_pack)?
        .build();

    // Build the server with the listener, router, layers, and state.
    let token = CancellationToken::new();
    let auth_secret = config.auth_secret
        .take()
        .map(|s| Bytes::copy_from_slice(s.as_bytes()));
    let server = Server::new(listener)
        .layer(proxy_layer)
        .layer(limiter_layer)
        .dispatch(Arc::new(router))
        .state(move |addr| {
            State::new(routes.clone(), *addr, auth_secret.clone(), config.auth_cookie_expiry)
        })
        .graceful_shutdown(token.child_token());

    // Run the server until completion.
    let mut serve_handle = tokio::spawn(server.serve());
    tokio::select! {
        result = &mut serve_handle => {
            result?;
        },
        _ = tokio::signal::ctrl_c() => {
            token.cancel();
            serve_handle.await?;
        },
    }
    Ok(())
}
