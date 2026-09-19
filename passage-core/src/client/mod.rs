//! The dialling half: one [`Connection`](crate::connection::Connection) over a socket we opened.
//!
//! [`Client`] is [`Server`](crate::server::Server) with the arrow turned around. It is built the
//! same way, takes the same [`Layer`](crate::router::Layer) stack and the same
//! [`Router`](crate::router::Router), and hands the socket to the same connection -- only the socket
//! comes from a [`Connector`] instead of a [`Listener`](crate::server::Listener), and there is
//! exactly one of it.
//!
//! What that changes:
//!
//! * The outcome is returned, not logged. A client has a caller waiting for it, and that caller is
//!   the one place that knows whether a failure matters.
//! * The dialling side speaks first, which is what [`Dispatcher::on_open`](crate::connection::Dispatcher::on_open)
//!   exists for. A client with no open hook sends nothing and waits forever.
//!
//! # Testing
//!
//! This is also the crate's test harness. [`Connected`] turns one half of a
//! [`tokio::io::duplex`] pair into a [`Connector`], so a client and a server can be run against each
//! other in-process, with no socket and no port.

mod client;
mod connector;
mod error;

pub use client::*;
pub use connector::*;
pub use error::*;
