//! The packets themselves, one module per protocol phase.
//!
//! A packet is a plain struct plus a [`Packet`](packet::Packet) impl that says what it is called,
//! which phase it belongs to, which ID it carries in which version, and how it reads and writes
//! itself. That is the whole contract: the [`router`](crate::router) infers everything else --
//! including which versions need a dispatch table of their own -- from the IDs a packet declares.
//!
//! Only the packets Passage actually speaks are here. This is not a complete protocol
//! implementation, and the phases after configuration have no packets at all: a transferred client
//! is the backend's from that point on.

pub mod configuration;
pub mod handshake;
pub mod login;
pub mod packet;
pub mod status;
