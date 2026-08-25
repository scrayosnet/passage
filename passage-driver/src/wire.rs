//! Bounds-checked primitives for the Minecraft wire format.
//!
//! Every read is checked against the bytes that are actually available *and* against an explicit
//! [`Limits`] policy. The two rules that hold everywhere in this module:
//!
//! * **Never allocate based on a number you have not received the bytes for.** A length prefix is
//!   validated against `remaining()` before it is used, so a 5 byte packet can never cause a
//!   gigabyte allocation.
//! * **Never let peer input reach an arithmetic operation that can panic.** No `as usize` on a
//!   possibly-negative value, no indexing without a check, no unchecked shifts.
//!
//! [`Reader`] borrows its buffer, so decoding a packet copies only what ends up in owned fields
//! (strings). Unknown packets cost nothing but the frame split.

use crate::error::{ProtocolError, Result};
use crate::version::ProtocolVersion;
use bytes::{BufMut, BytesMut};
use uuid::Uuid;

/// The size and length policy applied while decoding.
///
/// Limits are data, not constants, so a route can tighten them (e.g. a status-only route needs
/// tiny frames) without touching any codec.
#[derive(Copy, Clone, Debug)]
pub struct Limits {
    /// Maximum length of a single frame, excluding the length prefix.
    pub max_frame_len: usize,

    /// Maximum length of a string field in bytes.
    pub max_string_len: usize,

    /// Maximum number of elements in a length-prefixed array or byte blob.
    pub max_array_len: usize,

    /// Whether non-canonical (overlong) `VarInt`/`VarLong` encodings are rejected.
    ///
    /// Vanilla clients always encode minimally. Rejecting overlong forms removes a
    /// parser-differential surface, at the cost of being stricter than the reference
    /// implementation -- hence the switch.
    pub canonical_varints: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // Passage never legitimately receives large packets; the biggest is a status response
            // it sends itself.
            max_frame_len: 8 * 1024,
            // The protocol caps strings at 32767 UTF-16 code units, i.e. at most 3 bytes each plus
            // the prefix. Individual fields should use something far smaller.
            max_string_len: 32_767 * 3,
            max_array_len: 1024,
            canonical_varints: true,
        }
    }
}

/// A bounds-checked reader over a packet payload.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    limits: Limits,
}

impl<'a> Reader<'a> {
    /// Creates a reader over `buf`.
    #[must_use]
    pub fn new(buf: &'a [u8], limits: Limits) -> Self {
        Self {
            buf,
            pos: 0,
            limits,
        }
    }

    /// The limits this reader enforces.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The number of bytes consumed so far.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// The number of bytes left in the buffer.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Consumes exactly `n` bytes.
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let remaining = self.remaining();
        if remaining < n {
            return Err(ProtocolError::Eof {
                needed: n,
                remaining,
            }
            .into());
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Reads a single byte.
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Reads a boolean. Any non-zero byte is `true`, matching the vanilla implementation.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    /// Reads a big-endian `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// Reads a big-endian `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        let bytes = self.take(8)?;
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        Ok(i64::from_be_bytes(array))
    }

    /// Reads a big-endian `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(self.i64()? as u64)
    }

    /// Reads a UUID (two big-endian `u64`s).
    pub fn uuid(&mut self) -> Result<Uuid> {
        let bytes = self.take(16)?;
        let mut array = [0u8; 16];
        array.copy_from_slice(bytes);
        Ok(Uuid::from_bytes(array))
    }

    /// Reads a `VarInt`.
    ///
    /// Rejects encodings longer than five bytes, encodings whose last byte carries bits that do not
    /// fit into 32 bits, and (optionally) overlong encodings.
    pub fn var_int(&mut self) -> Result<i32> {
        const KIND: &str = "VarInt";
        let mut result: i32 = 0;
        for index in 0..5 {
            let byte = self.u8()?;
            let bits = i32::from(byte & 0b0111_1111);
            // The fifth byte only has four significant bits; anything else would silently wrap.
            if index == 4 && bits > 0b1111 {
                return Err(ProtocolError::VarIntTooLong { kind: KIND }.into());
            }
            result |= bits << (7 * index);
            if byte & 0b1000_0000 == 0 {
                if self.limits.canonical_varints && index > 0 && bits == 0 {
                    return Err(ProtocolError::VarIntNotCanonical { kind: KIND }.into());
                }
                return Ok(result);
            }
        }
        Err(ProtocolError::VarIntTooLong { kind: KIND }.into())
    }

    /// Reads a `VarLong`.
    ///
    /// Note the bound: a `VarLong` is up to **ten** bytes, because 64 bits do not divide into
    /// groups of seven. Stopping at nine truncates every value above `2^63 - 1` *and* leaves a byte
    /// in the stream, which desynchronises every following field.
    pub fn var_long(&mut self) -> Result<i64> {
        const KIND: &str = "VarLong";
        let mut result: i64 = 0;
        for index in 0..10 {
            let byte = self.u8()?;
            let bits = i64::from(byte & 0b0111_1111);
            // The tenth byte only has one significant bit.
            if index == 9 && bits > 0b1 {
                return Err(ProtocolError::VarIntTooLong { kind: KIND }.into());
            }
            result |= bits << (7 * index);
            if byte & 0b1000_0000 == 0 {
                if self.limits.canonical_varints && index > 0 && bits == 0 {
                    return Err(ProtocolError::VarIntNotCanonical { kind: KIND }.into());
                }
                return Ok(result);
            }
        }
        Err(ProtocolError::VarIntTooLong { kind: KIND }.into())
    }

    /// Reads a length prefix for `field`, rejecting negative values, values above `limit` and
    /// values that exceed the bytes left in the buffer.
    pub fn length(&mut self, field: &'static str, limit: usize) -> Result<usize> {
        let raw = self.var_int()?;
        if raw < 0 {
            return Err(ProtocolError::NegativeLength { field, value: raw }.into());
        }
        // `raw` is non-negative, so the cast is lossless on every target we support.
        let length = raw as usize;
        if length > limit {
            return Err(ProtocolError::LengthLimit {
                field,
                limit,
                actual: length,
            }
            .into());
        }
        let remaining = self.remaining();
        if length > remaining {
            return Err(ProtocolError::Eof {
                needed: length,
                remaining,
            }
            .into());
        }
        Ok(length)
    }

    /// Reads a length-prefixed byte slice, borrowed from the underlying buffer.
    pub fn bytes(&mut self, field: &'static str, limit: usize) -> Result<&'a [u8]> {
        let length = self.length(field, limit)?;
        self.take(length)
    }

    /// Reads a length-prefixed UTF-8 string.
    pub fn string(&mut self, field: &'static str, limit: usize) -> Result<String> {
        let bytes = self.bytes(field, limit)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ProtocolError::Utf8 { field }.into())
    }

    /// Borrows the rest of the buffer without consuming it.
    #[must_use]
    pub fn peek_rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    /// Consumes the rest of the buffer.
    pub fn rest(&mut self) -> &'a [u8] {
        let slice = &self.buf[self.pos..];
        self.pos = self.buf.len();
        slice
    }

    /// Asserts that the whole payload was consumed.
    ///
    /// Called by generated decoders. A packet with trailing bytes means our field list and the
    /// peer's disagree -- continuing would decode later packets against a wrong assumption.
    pub fn finish(&self, packet: &'static str) -> Result<()> {
        let remaining = self.remaining();
        if remaining > 0 {
            return Err(ProtocolError::TrailingBytes { packet, remaining }.into());
        }
        Ok(())
    }
}

/// A writer for the Minecraft wire format.
pub struct Writer<'a> {
    buf: &'a mut BytesMut,
}

impl<'a> Writer<'a> {
    /// Creates a writer that appends to `buf`.
    #[must_use]
    pub fn new(buf: &'a mut BytesMut) -> Self {
        Self { buf }
    }

    /// The number of bytes written so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether nothing has been written yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Writes a single byte.
    pub fn u8(&mut self, value: u8) {
        self.buf.put_u8(value);
    }

    /// Writes a boolean.
    pub fn bool(&mut self, value: bool) {
        self.buf.put_u8(u8::from(value));
    }

    /// Writes a big-endian `u16`.
    pub fn u16(&mut self, value: u16) {
        self.buf.put_u16(value);
    }

    /// Writes a big-endian `i64`.
    pub fn i64(&mut self, value: i64) {
        self.buf.put_i64(value);
    }

    /// Writes a big-endian `u64`.
    pub fn u64(&mut self, value: u64) {
        self.buf.put_u64(value);
    }

    /// Writes a UUID.
    pub fn uuid(&mut self, value: &Uuid) {
        self.buf.put_slice(value.as_bytes());
    }

    /// Writes a `VarInt`.
    pub fn var_int(&mut self, value: i32) {
        let mut remaining = value as u32;
        loop {
            let byte = (remaining & 0b0111_1111) as u8;
            remaining >>= 7;
            if remaining == 0 {
                self.buf.put_u8(byte);
                return;
            }
            self.buf.put_u8(byte | 0b1000_0000);
        }
    }

    /// Writes a `VarLong`.
    pub fn var_long(&mut self, value: i64) {
        let mut remaining = value as u64;
        loop {
            let byte = (remaining & 0b0111_1111) as u8;
            remaining >>= 7;
            if remaining == 0 {
                self.buf.put_u8(byte);
                return;
            }
            self.buf.put_u8(byte | 0b1000_0000);
        }
    }

    /// Writes a length-prefixed byte slice.
    pub fn bytes(&mut self, value: &[u8]) {
        self.var_int(value.len() as i32);
        self.buf.put_slice(value);
    }

    /// Writes a length-prefixed string.
    pub fn string(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    /// Writes raw bytes without a length prefix.
    pub fn raw(&mut self, value: &[u8]) {
        self.buf.put_slice(value);
    }
}

/// A value that can be read from and written to the wire.
///
/// Implementations are per *wire type*, not per Rust type: `i32` has no `Wire` impl because the
/// protocol has three different encodings for it. [`VarInt`] and friends make the choice explicit
/// at the field declaration, where it belongs.
pub trait Wire: Sized {
    /// Decodes the value.
    fn read(reader: &mut Reader<'_>, version: ProtocolVersion, field: &'static str)
    -> Result<Self>;

    /// Encodes the value.
    fn write(&self, writer: &mut Writer<'_>, version: ProtocolVersion);
}

/// A `VarInt`-encoded `i32`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarInt(pub i32);

/// A `VarLong`-encoded `i64`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VarLong(pub i64);

impl Wire for VarInt {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        Ok(Self(reader.var_int()?))
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.var_int(self.0);
    }
}

impl Wire for VarLong {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        Ok(Self(reader.var_long()?))
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.var_long(self.0);
    }
}

impl Wire for bool {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.bool()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.bool(*self);
    }
}

impl Wire for u8 {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.u8()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.u8(*self);
    }
}

impl Wire for u16 {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.u16()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.u16(*self);
    }
}

impl Wire for i64 {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.i64()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.i64(*self);
    }
}

impl Wire for u64 {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.u64()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.u64(*self);
    }
}

impl Wire for Uuid {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, _field: &'static str) -> Result<Self> {
        reader.uuid()
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.uuid(self);
    }
}

impl Wire for String {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, field: &'static str) -> Result<Self> {
        let limit = reader.limits().max_string_len;
        reader.string(field, limit)
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.string(self);
    }
}

/// A length-prefixed blob of raw bytes.
///
/// A distinct type rather than `Vec<u8>` so that a byte blob and an array of `u8` fields cannot be
/// confused at the declaration site.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ByteArray(pub Vec<u8>);

impl Wire for ByteArray {
    fn read(reader: &mut Reader<'_>, _v: ProtocolVersion, field: &'static str) -> Result<Self> {
        let limit = reader.limits().max_array_len;
        Ok(Self(reader.bytes(field, limit)?.to_vec()))
    }

    fn write(&self, writer: &mut Writer<'_>, _v: ProtocolVersion) {
        writer.bytes(&self.0);
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn read(
        reader: &mut Reader<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self> {
        // `length` also refuses counts larger than the bytes left in the frame, so a claimed
        // element count can never make us reserve memory the peer has not paid for.
        let limit = reader.limits().max_array_len;
        let count = reader.length(field, limit)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(T::read(reader, version, field)?);
        }
        Ok(values)
    }

    fn write(&self, writer: &mut Writer<'_>, version: ProtocolVersion) {
        writer.var_int(self.len() as i32);
        for value in self {
            value.write(writer, version);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    fn reader(bytes: &[u8]) -> Reader<'_> {
        Reader::new(bytes, Limits::default())
    }

    #[test]
    fn var_int_roundtrips() {
        for value in [0, 1, 127, 128, 255, 2_097_151, i32::MAX, -1, i32::MIN] {
            let mut buf = BytesMut::new();
            Writer::new(&mut buf).var_int(value);
            assert_eq!(reader(&buf).var_int().expect("decodes"), value, "{value}");
        }
    }

    #[test]
    fn var_long_roundtrips_ten_byte_values() {
        for value in [0, 1, i64::MAX, -1, i64::MIN] {
            let mut buf = BytesMut::new();
            Writer::new(&mut buf).var_long(value);
            let decoded = reader(&buf).var_long().expect("decodes");
            assert_eq!(decoded, value, "{value} encoded as {} bytes", buf.len());
        }
        // -1 needs all ten bytes; a nine byte bound silently truncates it.
        let mut buf = BytesMut::new();
        Writer::new(&mut buf).var_long(-1);
        assert_eq!(buf.len(), 10);
    }

    #[test]
    fn var_int_rejects_overlong_encodings() {
        // Six continuation bytes.
        let err = reader(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01])
            .var_int()
            .expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::VarIntTooLong { .. })
        ));

        // Five bytes whose last byte overflows 32 bits.
        let err = reader(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
            .var_int()
            .expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::VarIntTooLong { .. })
        ));

        // Non-canonical zero.
        let err = reader(&[0x80, 0x00]).var_int().expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::VarIntNotCanonical { .. })
        ));
    }

    #[test]
    fn negative_length_is_rejected_before_allocating() {
        // 0xFF 0xFF 0xFF 0xFF 0x0F decodes to -1. Cast to `usize` this is 18446744073709551615.
        let mut r = reader(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
        let err = r.string("server_address", 32_767).expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::NegativeLength { value: -1, .. })
        ));
    }

    #[test]
    fn huge_length_is_rejected_before_allocating() {
        // A ten byte packet claiming a 2 GiB string.
        let mut buf = BytesMut::new();
        Writer::new(&mut buf).var_int(i32::MAX);
        buf.extend_from_slice(b"abcde");
        let err = reader(&buf)
            .string("server_address", 32_767)
            .expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::LengthLimit { .. })
        ));
    }

    #[test]
    fn length_beyond_the_buffer_is_eof_not_an_allocation() {
        let mut buf = BytesMut::new();
        Writer::new(&mut buf).var_int(4096);
        buf.extend_from_slice(b"abc");
        let err = reader(&buf)
            .bytes("payload", usize::MAX)
            .expect_err("must reject");
        assert!(matches!(err, Error::Protocol(ProtocolError::Eof { .. })));
    }

    #[test]
    fn string_roundtrips() {
        let mut buf = BytesMut::new();
        Writer::new(&mut buf).string("mc.justchunks.net");
        assert_eq!(
            reader(&buf).string("host", 64).expect("decodes"),
            "mc.justchunks.net"
        );
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut r = reader(&[0x01, 0x02]);
        r.u8().expect("reads");
        assert!(matches!(
            r.finish("Demo").expect_err("must reject"),
            Error::Protocol(ProtocolError::TrailingBytes { remaining: 1, .. })
        ));
    }
}
