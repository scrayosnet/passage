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

mod error;
mod options;
mod property;
mod reader;
mod writer;

pub use self::{
    error::Result as WireResult, error::WireError, options::Options, property::Property,
    reader::Reader, writer::Writer,
};
