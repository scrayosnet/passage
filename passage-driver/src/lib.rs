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
//!   once, with an ID table keyed by protocol version ([`IDS`](packet::Packet::IDS)) and fields
//!   gated by a version comparison ([`at_least`](version::ProtocolVersion::at_least)). One type
//!   serves every version, and because the table is *data*, [`Router`](router::Router) reads the
//!   thresholds out of it and builds one dispatch table per version at which something actually
//!   changes -- so no list of supported versions exists to be forgotten. Codecs are written out
//!   rather than generated, which is what lets a field name its own length limit and a decoder
//!   produce a domain type instead of a raw `VarInt`.
//! * **Handlers are registered, not implemented.** [`RouterBuilder::on`](router::RouterBuilder::on)
//!   takes a typed handler per packet -- `.on::<LoginStart>(on_login_start)`. Adding a packet does
//!   not widen a trait, so it does not break anything that already exists. Tables are built once at startup, so an ID collision is a boot
//!   failure and a connection allocates nothing.
//! * **Handlers are synchronous, and everything they do is an operation.** A handler reads
//!   [`Ctx::state`](conn::Ctx::state) and queues [`Op`](conn::Op)s -- a packet, a state change, a
//!   phase change, a background task. The connection drains them in order, which is what makes
//!   "record the profile, then announce it" and "send this, then switch to encryption" mean what
//!   they say.
//! * **One task owns everything.** The socket, the state, the phase and the version all live on the
//!   [`Connection`](conn::Connection). No locks, no atomics, and no way to observe a half-applied
//!   change.
//! * **The connection does not know what a router is.** It reaches dispatch through
//!   [`Dispatcher`](conn::Dispatcher), a trait declared by the side that uses it and implemented by
//!   [`RouterDispatcher`](router::RouterDispatcher). So the table lives with the thing that owns
//!   routing, a connection can be driven by a test double or a recorder, and the two halves can be
//!   read separately.
//! * **Waiting is explicit.** [`Ctx::exclusive`](conn::Ctx::exclusive) says the peer must stay quiet
//!   until a future resolves -- so a packet that arrives anyway is reported rather than replayed,
//!   and a hangup mid-authentication is noticed at once. [`Ctx::spawn`](conn::Ctx::spawn) is for
//!   work that must overlap with further traffic, and runs on the connection's own task;
//!   [`Ctx::detach`](conn::Ctx::detach) is for work that might block and must not stop it.
//! * **Ending is something you can answer.** A connection that ends for a reason nobody asked for
//!   -- a failure, a deadline, a shutdown -- goes to [`Dispatcher::on_error`](conn::Dispatcher::on_error)
//!   first, with everything already queued still on its way out. That is where a disconnect message
//!   comes from, and the reason a refused login no longer looks to the player like a crash.
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
//! | the dispatch tables, one per version | a [`RouterDispatcher`](router::RouterDispatcher), holding the table for the version this connection negotiated |
//!
//! The left column is immutable and shared behind an [`Arc`](std::sync::Arc); nothing in the right
//! column is shared with anything, which is why none of it needs a lock.
//! [`Server`](server::Server) is the bridge: it accepts sockets and builds the right column for
//! each one.
//!
//! # Example
//!
//! ```no_run
//! use passage_driver::demo::server::{Session, log_completion, router};
//! use passage_driver::server::Server;
//! use std::sync::Arc;
//! use std::time::Duration;
//! use tokio::net::TcpListener;
//! use tokio_util::sync::CancellationToken;
//!
//! # async fn run(shutdown: CancellationToken) -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("0.0.0.0:25565").await?;
//!
//! Server::builder()
//!     .listener(listener)
//!     // Built once at startup and shared by every connection. It names no protocol versions: the
//!     // packets carry their own, and the dispatch tables follow.
//!     .dispatch(Arc::new(router()?))
//!     // Called once per accepted socket, because the state is that connection's alone.
//!     .state(|addr| Session {
//!         peer: Some(*addr),
//!         ..Session::default()
//!     })
//!     .tick_interval(Duration::from_secs(16))
//!     .max_lifetime(Some(Duration::from_secs(60)))
//!     .max_connections(10_000)
//!     .graceful_shutdown(shutdown)
//!     .on_finish(log_completion)
//!     .await;
//! # Ok(())
//! # }
//! ```
//!
//! Everything is built the same way: [`Router::builder`](router::Router::builder),
//! [`Server::builder`](server::Server::builder),
//! [`Connection::builder`](conn::Connection::builder). The three the server cannot do without --
//! a listener, something to dispatch to, a state factory -- are typestate, so `.await` does not
//! exist until all three are set.
//!
//! One connection at a time, without the accept loop, is
//! [`Connection::builder`](conn::Connection::builder) -- what the server uses per socket, and what
//! the tests drive over a socket pair.

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
