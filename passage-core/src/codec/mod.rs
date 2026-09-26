//! The frame codec: length-prefixed frames, optional in-place encryption.
//!
//! The codec is deliberately *phase- and version-agnostic*. It turns a byte stream into [`Frame`]s
//! (a packet ID plus its still-undecoded payload) and back. Nothing here knows what a packet is,
//! which is what lets the same codec serve a client, a server, and a test harness.

mod cipher;
mod codec;
mod error;

pub use cipher::{Aes128Cfb8, Cipher, NoCipher, SECRET_LEN};
pub use codec::{Frame, FrameCodec, UNKNOWN_PACKET_NAME};
pub use error::{CodecError, Result};
