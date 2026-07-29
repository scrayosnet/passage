//! An opinionated implementation of the Minecraft: Java Edition protocol, tailored to routing players
//! with the [transfer packet](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Transfer_(configuration))
//! introduced in Minecraft 1.20.5.
//!
//! # Overview
//!
//! The [`listener`] accepts TCP connections and hands each one to a [`connection::Connection`], which
//! drives the protocol state machine `Handshake → Status | Login → Configuration → Transfer`. The
//! hostname from the handshake is matched against the configured [`routes::Routes`]; the matching
//! [`routes::Route`] supplies the adapters that answer status pings, authenticate the player, localize
//! disconnect messages and discover the transfer target. After the transfer packet has been sent, the
//! connection is dropped -- no state about the player is retained.
//!
//! # Modules
//!
//! - [`config`] -- protocol-level configuration and its defaults
//! - [`connection`] -- the per-connection protocol state machine
//! - [`cookie`] -- the signed authentication cookie and the unsigned session cookie
//! - [`crypto`] -- RSA key generation, AES-CFB8 encryption and the Minecraft SHA-1 variant
//! - [`listener`] -- the TCP listener, including PROXY protocol support
//! - [`rate_limiter`] -- per-IP connection rate limiting
//! - [`routes`] -- hostname matching and the per-route adapter set

pub mod config;
pub mod connection;
pub mod cookie;
pub mod crypto;
pub mod error;
pub mod listener;
pub mod metrics;
pub mod rate_limiter;
pub mod routes;

pub use error::*;
