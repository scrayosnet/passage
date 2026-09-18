//! The accept loop: one [`Connection`](crate::connection::Connection) per accepted socket.
//!
//! Deliberately thin: accept, build the state for that peer, hand both to a connection, and track
//! the task so shutdown can wait for it. Only the accept itself happens in the loop -- the layers,
//! the state factory and the protocol all run on the connection's own task, because all of them can
//! wait on the peer and none may hold up the next accept.
//!
//! State is *per connection*, not shared, so [`Server::state`] takes a factory and calls it once per
//! socket. Anything genuinely shared belongs in a captured [`Arc`](std::sync::Arc).
//!
//! Everything between the accept and the protocol is a [`Layer`]: a PROXY header, a TLS handshake, a
//! rate limiter. All the same shape, all set with [`Server::layer`], and this module ships none of
//! them -- a layer belongs next to the thing it implements.

mod error;
mod layer;
mod listener;
mod server;

pub use layer::*;
pub use listener::*;
pub use server::*;
