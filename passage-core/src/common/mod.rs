//! The vocabulary the packets are written in: the types that appear inside more than one of them.
//!
//! A [`ProtocolVersion`] and a [`Phase`] are what the connection is; the rest are field types --
//! enums the client sends, a [`Profile`], a [`TextComponent`] -- that implement
//! [`Property`](crate::wire::Property) so a packet can read and write them by name. Nothing here
//! is a packet, and nothing here knows which packet it ends up in.

mod common;
mod component;
mod phase;
mod profile;
mod version;

pub use common::*;
pub use component::*;
pub use phase::*;
pub use profile::*;
pub use version::*;
