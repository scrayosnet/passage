//! Typed packet registration with erased dispatch.
//!
//! Registration is generic (`.on::<LoginStart>(on_login_start)`), so the handler receives a decoded
//! packet and a wrong pairing does not compile. Storage is erased, so dispatch is a table lookup and
//! one virtual call -- and adding a packet does not widen any trait.
//!
//! # Tables are built once, one per *change*
//!
//! Which versions get a table is not something the caller has to know. Every packet declares its IDs
//! as data ([`Packet::IDS`](crate::Packet::IDS)), so [`Table::breakpoints`] reads the *thresholds*
//! out of them -- the versions at which some packet appeared, vanished or was renumbered -- and
//! builds one table per threshold. Between two thresholds nothing about dispatch differs.
//!
//! Below the lowest threshold sits the version-independent table: the packets whose IDs start at
//! [`ProtocolVersion::UNKNOWN`](crate::ProtocolVersion::UNKNOWN), which is enough to answer a status
//! ping and refuse a login. Versions that cannot be ordered at all -- snapshots, negative numbers --
//! get that table too.
//!
//! # How a connection reaches this
//!
//! Not directly. A [`Connection`](crate::connection::Connection) depends on the
//! [`Dispatcher`](crate::connection::Dispatcher) trait, and [`RouterDispatcher`] is the
//! implementation that dispatches to a router. That is why no type in `connection` names a
//! [`Router`].

mod dispatch;
mod error;
pub mod layer;
mod router;
mod table;

pub use dispatch::*;
pub use error::*;
pub use layer::*;
pub use router::*;
pub use table::*;
