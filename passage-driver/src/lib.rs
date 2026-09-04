//! A general-purpose backbone for the Minecraft (Java) protocol.
//!
//! The driver does four things: it frames packets, dispatches them to handlers, ticks, and shuts
//! down. It knows nothing about authentication, routing, resource packs or transfers -- those are
//! handlers written on top of it. Layered like this:
//!
//! ```text
//! passage             configuration, adapters, wiring
//! passage-server      the Minecraft server flow as handlers  (see `demo::server`)
//! passage-driver      framing, dispatch, ticks, shutdown, errors
//! ```
//!
//! # Design in one page
//!
//! * **Packets carry their own version mapping.** A packet implements [`Packet`](packet::Packet)
//!   once, with an ID table keyed by protocol version ([`ids`](packet::ids)) and fields gated by a
//!   version comparison ([`at_least`](version::ProtocolVersion::at_least)). One type serves every
//!   version;
//!   [`Router`](router::Router) derives the decode table from the same declaration, so there is no
//!   second place to forget. Codecs are written out rather than generated, which is what lets a
//!   field name its own length limit and a decoder produce a domain type instead of a raw `VarInt`.
//! * **Handlers are registered, not implemented.** [`RouterBuilder::on`](router::RouterBuilder::on)
//!   takes a typed handler per packet. Adding a packet does not widen a trait, so it does not break
//!   anything that already exists. Tables are built once at startup, so an ID collision is a boot
//!   failure and a connection allocates nothing.
//! * **Handlers are synchronous, and everything they do is an operation.** A handler reads
//!   [`Ctx::state`](conn::Ctx::state) and queues [`Op`](conn::Op)s -- a packet, a state change, a
//!   phase change, a background task. The driver drains them in order, which is what makes "record
//!   the profile, then announce it" and "send this, then switch to encryption" mean what they say.
//! * **One task owns everything.** The socket, the state, the phase and the version all live on the
//!   driver. No locks, no atomics, and no way to observe a half-applied change.
//! * **Waiting is explicit.** [`Ctx::exclusive`](conn::Ctx::exclusive) says the peer must stay quiet
//!   until a future resolves -- so a packet that arrives anyway is reported rather than replayed,
//!   and a hangup mid-authentication is noticed at once. [`Ctx::spawn`](conn::Ctx::spawn) is for
//!   work that must overlap with further traffic.
//! * **Errors say who is to blame.** [`Class`](error::Class) separates a scanner's malformed packet
//!   from our own bug, handlers classify their own failures, and completing a connection is [`Ok`],
//!   never an error variant. Wiring mistakes are [`BuildError`](error::BuildError)s and cannot
//!   reach a connection at all.
//!
//! # Example
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, router};
//! use passage_driver::driver::{Driver, DriverConfig};
//! use std::sync::Arc;
//! use std::time::Duration;
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn run(io: tokio::io::DuplexStream) -> Result<(), Box<dyn std::error::Error>> {
//! // Built once at startup and shared by every connection.
//! let router = Arc::new(router()?);
//!
//! let config = DriverConfig {
//!     tick_interval: Some(Duration::from_secs(16)),
//!     max_lifetime: Some(Duration::from_secs(60)),
//!     max_idle: Some(Duration::from_secs(30)),
//!     ..DriverConfig::default()
//! };
//!
//! let (driver, _handle) = Driver::new(
//!     io,
//!     router,
//!     Session::default(),
//!     config,
//!     CancellationToken::new(),
//! );
//! let completion = driver.run().await?;
//! # let _ = completion;
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod conn;
pub mod driver;
pub mod error;
pub mod packet;
pub mod router;
pub mod version;
pub mod wire;

#[cfg(feature = "demo")]
pub mod demo;
