//! Bounds-checked primitives for the Minecraft wire format.
//!
//! Every read is checked against the bytes that are actually available *and* against an explicit
//! limit, so a length prefix can never cause an allocation the peer has not paid for. Every field
//! names its own bound where it is read (`r.string("server_address", 255)`), because a hostname and
//! a chat component have nothing to do with each other; [`Options::max_frame_len`] bounds all of
//! them transitively.
//!
//! Encodings are chosen by the call, not by the type: the protocol has more than one encoding for
//! an `i32`, so a decoder says `r.var_int(..)` or `r.i32(..)` and the choice is visible where it is
//! made.
//!
//! A [`Reader`] owns the frame it reads, so a field a packet keeps whole is a slice of that frame
//! rather than a copy of it: `Bytes` for bytes, and [`ByteString`](bytestring::ByteString) -- bytes
//! that are known to be UTF-8 -- for strings. Decoding a packet therefore allocates only for what
//! it reshapes, which is NBT and JSON, and what a handler keeps holds its frame alive.

mod error;
mod options;
mod property;
mod reader;
mod writer;

pub use self::{
    error::Result as WireResult, error::WireError, options::Options, property::Property,
    reader::Reader, reader::read_var_int, writer::Writer,
};

// The two types every packet field is made of, so that writing one does not mean depending on the
// exact versions of `bytes` and `bytestring` this crate resolved.
pub use bytes::Bytes;
pub use bytestring::ByteString;
