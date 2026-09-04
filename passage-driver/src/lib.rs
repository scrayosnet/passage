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
//!   phase change, a background task. The connection drains them in order, which is what makes
//!   "record the profile, then announce it" and "send this, then switch to encryption" mean what
//!   they say.
//! * **One task owns everything.** The socket, the state, the phase and the version all live on the
//!   [`Connection`](conn::Connection). No locks, no atomics, and no way to observe a half-applied
//!   change.
//! * **Waiting is explicit.** [`Ctx::exclusive`](conn::Ctx::exclusive) says the peer must stay quiet
//!   until a future resolves -- so a packet that arrives anyway is reported rather than replayed,
//!   and a hangup mid-authentication is noticed at once. [`Ctx::spawn`](conn::Ctx::spawn) is for
//!   work that must overlap with further traffic.
//! * **Errors say who is to blame.** [`Class`](error::Class) separates a scanner's malformed packet
//!   from our own bug, handlers classify their own failures, and completing a connection is [`Ok`],
//!   never an error variant. Wiring mistakes are [`BuildError`](error::BuildError)s and cannot
//!   reach a connection at all.
//!
//! # What is static and what is per connection
//!
//! The two halves of the crate, and the reason the vocabulary is worth learning:
//!
//! | Static, built once            | One per accepted socket                  |
//! |-------------------------------|------------------------------------------|
//! | [`Router`](router::Router)    | [`Connection`](conn::Connection)         |
//! | [`ConnectionConfig`](conn::ConnectionConfig) | [`ConnectionHandle`](conn::ConnectionHandle) |
//! | the handlers themselves       | the state `S`, and a [`Ctx`](conn::Ctx) per handler call |
//!
//! The left column is immutable and shared behind an [`Arc`](std::sync::Arc); nothing in the right
//! column is shared with anything, which is why none of it needs a lock.
//! [`serve`](server::serve) is the bridge: it accepts sockets and builds the right column for each
//! one.
//!
//! # Example
//!
//! ```no_run
//! use passage_driver::conn::ConnectionConfig;
//! use passage_driver::demo::server::{Session, log_completion, router};
//! use passage_driver::server::serve;
//! use std::time::Duration;
//! use tokio::net::TcpListener;
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn run(shutdown: CancellationToken) -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("0.0.0.0:25565").await?;
//!
//! // Built once at startup and shared by every connection.
//! let router = router()?;
//!
//! let config = ConnectionConfig {
//!     tick_interval: Some(Duration::from_secs(16)),
//!     max_lifetime: Some(Duration::from_secs(60)),
//!     max_idle: Some(Duration::from_secs(30)),
//!     ..ConnectionConfig::default()
//! };
//!
//! // Built once per accepted socket.
//! serve(listener, router, |addr| Session {
//!     peer: Some(*addr),
//!     ..Session::default()
//! })
//! .config(config)
//! .with_graceful_shutdown(shutdown)
//! .on_finish(log_completion)
//! .await;
//! # Ok(())
//! # }
//! ```
//!
//! One connection at a time, without the accept loop, is
//! [`Connection::new`](conn::Connection::new) -- what `serve` calls per socket, and what the tests
//! drive over a socket pair.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod conn;
pub mod error;
pub mod packet;
pub mod router;
pub mod server;
pub mod version;
pub mod wire;

#[cfg(feature = "demo")]
pub mod demo;
