//! A worked example: a packet set and a server built on the driver.
//!
//! This module exists to demonstrate the proposals in `docs/`, and to keep them honest -- every
//! claim about ergonomics or safety in those documents is exercised by the tests here and in
//! `tests/`. It is not the eventual `passage-server`; it covers just enough of the protocol
//! (handshake, status, login, configuration) to show every mechanism at work.
//!
//! It sits behind the `demo` feature, which is on by default only because the documentation points
//! at it. Turn it off (`default-features = false`) and none of this is compiled into the binary.

pub mod packets;
pub mod server;
