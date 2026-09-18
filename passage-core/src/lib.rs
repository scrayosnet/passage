//! A general-purpose backbone for the Minecraft (Java) protocol.
//!
//! The crate does four things: it frames packets, dispatches them to handlers, ticks, and shuts
//! down. It knows nothing about authentication, routing, resource packs or transfers -- those are
//! handlers written on top of it.
//!
//! | Module              | Role                                                              |
//! |---------------------|-------------------------------------------------------------------|
//! | [`wire`]            | bounds-checked primitives for the Minecraft wire format           |
//! | [`codec`]           | length-prefixed framing and optional in-place encryption          |
//! | [`router`]          | typed packet registration with erased dispatch                    |
//! | [`connection`]      | one socket: the state, the operation queue, the loop              |
//! | [`server`]          | the accept loop, listeners and layers                             |
//! | [`client`]          | the dialling half, and the in-process harness the tests use       |
//!
//! Packet identity lives at the crate root rather than in a module of its own, because a packet is
//! named by all four of [`Packet`], [`Phase`], [`Direction`] and [`ProtocolVersion`] at once.
//!
//! Two halves are worth keeping apart while reading. A [`Router`](router::Router) is built once at
//! startup and shared by every connection behind an [`Arc`](std::sync::Arc); a
//! [`Connection`](connection::Connection) is created per accepted socket and owns everything
//! mutable. Nothing in the second half is shared, which is why none of it needs a lock.

#![deny(unsafe_code)]
#![warn(missing_docs)]
// Each module keeps its centrepiece in a file of its own name (`codec/codec.rs`, `router/router.rs`)
// so that `mod.rs` stays the module's documentation and re-export list. That is the layout, not an
// oversight.
#![allow(clippy::module_inception)]

mod direction;
mod packet;
mod phase;
mod version;

pub mod client;
pub mod codec;
pub mod connection;
pub mod router;
pub mod server;
pub mod wire;

pub use direction::Direction;
pub use packet::{Packet, check_ids_unordered, ids};
pub use phase::Phase;
pub use version::{ProtocolVersion, versions};
