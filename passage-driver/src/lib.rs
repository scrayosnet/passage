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
//! * **Packets carry their own version mapping.** [`packet!`] declares a packet once, with an ID
//!   table keyed by protocol version and fields that may be gated behind a named
//!   [`Feature`](version::Feature). One type serves every version;
//!   [`Router`](router::Router) derives the decode table from the same declaration, so there is no
//!   second place to forget.
//! * **Handlers are registered, not implemented.** [`Router::on`](router::Router::on) takes a typed
//!   handler per packet. Adding a packet does not widen a trait, so it does not break anything that
//!   already exists.
//! * **Handlers are synchronous unless they need not to be.** [`Flow`](flow::Flow) lets a handler
//!   return a value or a future; only the latter allocates.
//! * **One task owns the socket.** Handlers queue [`Op`](conn::Op)s, and the driver drains them
//!   first, in order -- which is what makes "enable encryption after this packet" correct rather
//!   than lucky.
//! * **Errors say who is to blame.** [`Class`](error::Class) separates a scanner's malformed packet
//!   from our own bug, and completing a connection is [`Ok`], never an error variant.
//!
//! # Example
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, router};
//! use passage_driver::driver::{Driver, DriverConfig};
//! use passage_driver::packet::Phase;
//! use std::sync::Arc;
//! use std::time::Duration;
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn run(io: tokio::io::DuplexStream) -> passage_driver::error::Result<()> {
//! let router = Arc::new(router());
//! let config = DriverConfig {
//!     tick_interval: Some(Duration::from_secs(16)),
//!     ..DriverConfig::default()
//! };
//!
//! let (driver, _handle) = Driver::new(
//!     io,
//!     router,
//!     Session::default(),
//!     config,
//!     CancellationToken::new(),
//! )?;
//! let completion = driver.run().await?;
//! # let _ = (completion, Phase::Handshake);
//! # Ok(())
//! # }
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod codec;
pub mod conn;
pub mod demo;
pub mod driver;
pub mod error;
pub mod flow;
pub mod packet;
pub mod router;
pub mod version;
pub mod wire;
