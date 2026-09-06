//! Bounds-checked primitives for the Minecraft wire format.
//!
//! Every read is checked against the bytes that are actually available *and* against an explicit
//! limit. The three rules that hold everywhere in this module:
//!
//! * **Never allocate based on a number you have not received the bytes for.** A length prefix is
//!   validated against `remaining()` before it is used, so a 5 byte packet can never cause a
//!   gigabyte allocation.
//! * **Never let peer input reach an arithmetic operation that can panic.** No `as usize` on a
//!   possibly-negative value, no indexing without a check, no unchecked shifts.
//! * **Never emit a length that disagrees with its payload.** The outbound path checks its casts
//!   too: a wrapped length prefix is the hardest kind of protocol bug to diagnose from the other
//!   end.
//!
//! [`Reader`] borrows its buffer, so decoding a packet copies only what ends up in owned fields
//! (strings). Unknown packets cost nothing but the frame split.
//!
//! # Encodings are chosen by the call, not by the type
//!
//! There is no `Wire` impl for `i32`, because the protocol has more than one encoding for it: a
//! `VarInt` in most packets, four fixed big-endian bytes in others. A decoder says `r.var_int()` or
//! `r.i32()`, and the choice is visible at the point it is made -- an impl keyed on the type could
//! only pick one and be wrong half the time. [`Wire`] exists only for *composite* values that
//! appear in more than one packet.
//!
//! The same distinction runs through the two kinds of absent value. [`Reader::optional`] is the
//! protocol's `Optional` -- a boolean on the wire, and the peer's choice. [`Reader::gated`] is a
//! version-gated field, which costs no bytes and is ours. They produce the same `Option<T>` from
//! entirely different bytes, so they are separate calls rather than one with a flag.

use crate::error::{InternalError, ProtocolError, Result};
use crate::version::ProtocolVersion;
use bytes::{BufMut, BytesMut};
use uuid::Uuid;

/// The connection-wide size policy.
///
/// Limits are data, not constants, so a route can tighten them (e.g. a status-only route needs
/// tiny frames) without touching any codec.
///
/// There is deliberately no "maximum string length" or "maximum array length" here. Every field
/// names its own bound where it is read (`r.string("server_address", 255)`), because a hostname and
/// a chat component have nothing to do with each other, and one shared number for both is a limit
/// that is simultaneously too tight and far too loose. [`Limits::max_frame_len`] bounds all of them
/// transitively: no field can be longer than the frame that carries it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Maximum length of a single frame, excluding the length prefix. Enforced in both directions.
    pub max_frame_len: usize,

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
            // Sized by the biggest thing a server legitimately *sends*, which is a status response
            // carrying a favicon -- base64 of a 64x64 PNG, plus a MOTD and a sample. The vanilla
            // client caps that JSON at 32,767 characters, so a limit below it would refuse ordinary
            // content: an earlier 8 KiB default did exactly that, as our own `OversizedFrame`.
            // Still 64x under vanilla's own inbound cap, and it bounds every field transitively.
            max_frame_len: 32 * 1024,
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

    /// Reads a signed byte.
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }

    /// Reads a boolean. Any non-zero byte is `true`, matching the vanilla implementation.
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    /// Reads a big-endian `u16`.
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `i16`.
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `i32`.
    ///
    /// Not the same encoding as [`var_int`](Reader::var_int), and the protocol uses both: block
    /// coordinates and entity IDs are `VarInt`s, while a chunk's `x`/`z` are four fixed bytes.
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `i64`.
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `u64`.
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `f32`.
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_be_bytes(self.fixed()?))
    }

    /// Reads a big-endian `f64`.
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_be_bytes(self.fixed()?))
    }

    /// Consumes exactly `N` bytes as an array, for the fixed-width numbers above.
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut array = [0u8; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    /// Reads a UUID (two big-endian `u64`s).
    pub fn uuid(&mut self) -> Result<Uuid> {
        Ok(Uuid::from_bytes(self.fixed()?))
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

    /// Reads a length-prefixed array of composite values.
    ///
    /// The element count is checked against the bytes left in the frame first, so a claimed count
    /// can never make us reserve memory the peer has not paid for.
    pub fn array<T: Wire>(
        &mut self,
        field: &'static str,
        limit: usize,
        version: ProtocolVersion,
    ) -> Result<Vec<T>> {
        let count = self.length(field, limit)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(T::read(self, version)?);
        }
        Ok(values)
    }

    /// Reads a value that only exists once `condition` holds, and [`None`] otherwise.
    ///
    /// This is the read half of a version-gated field. It is a plain `if` written once, so that
    /// `decode` bodies stay a flat list of fields:
    ///
    /// ```ignore
    /// session_id: r.gated(version.at_least(versions::V26_2), Reader::uuid)?,
    /// ```
    ///
    /// It is **not** the protocol's `Optional`, which is a boolean on the wire -- see
    /// [`optional`](Reader::optional). The two produce the same `Option<T>` from entirely different
    /// bytes: this one reads nothing at all when the version does not have the field, and the peer
    /// cannot influence it.
    pub fn gated<T>(
        &mut self,
        condition: bool,
        read: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<Option<T>> {
        if condition {
            read(self).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reads a value preceded by a boolean saying whether it is there.
    ///
    /// This is the protocol's `Optional`: one byte, then the value if that byte was set. Whether
    /// the field is present is the *peer's* choice, which is the whole difference from
    /// [`gated`](Reader::gated), where it is the version's.
    ///
    /// ```ignore
    /// payload: r.optional(|r| r.bytes("payload", MAX_COOKIE_LEN))?,
    /// ```
    pub fn optional<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T>) -> Result<Option<T>> {
        let present = self.bool()?;
        self.gated(present, read)
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
    /// The [`Router`](crate::router::Router) calls this after every decode, so no individual
    /// decoder has to remember to. A packet with trailing bytes means our field list and the peer's
    /// disagree -- continuing would decode later packets against a wrong assumption.
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
    packet: &'static str,
    limits: Limits,
}

impl<'a> Writer<'a> {
    /// Creates a writer that appends to `buf` on behalf of `packet`.
    ///
    /// The packet name and limits are carried so that a length that cannot be encoded is reported
    /// against the packet that caused it, rather than wrapping silently.
    #[must_use]
    pub fn new(buf: &'a mut BytesMut, packet: &'static str, limits: Limits) -> Self {
        Self {
            buf,
            packet,
            limits,
        }
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

    /// Writes a signed byte.
    pub fn i8(&mut self, value: i8) {
        self.buf.put_i8(value);
    }

    /// Writes a boolean.
    pub fn bool(&mut self, value: bool) {
        self.buf.put_u8(u8::from(value));
    }

    /// Writes a big-endian `u16`.
    pub fn u16(&mut self, value: u16) {
        self.buf.put_u16(value);
    }

    /// Writes a big-endian `i16`.
    pub fn i16(&mut self, value: i16) {
        self.buf.put_i16(value);
    }

    /// Writes a big-endian `i32`. Not the same encoding as [`var_int`](Writer::var_int).
    pub fn i32(&mut self, value: i32) {
        self.buf.put_i32(value);
    }

    /// Writes a big-endian `i64`.
    pub fn i64(&mut self, value: i64) {
        self.buf.put_i64(value);
    }

    /// Writes a big-endian `u64`.
    pub fn u64(&mut self, value: u64) {
        self.buf.put_u64(value);
    }

    /// Writes a big-endian `f32`.
    pub fn f32(&mut self, value: f32) {
        self.buf.put_f32(value);
    }

    /// Writes a big-endian `f64`.
    pub fn f64(&mut self, value: f64) {
        self.buf.put_f64(value);
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

    /// Writes a length prefix, refusing lengths that do not fit a frame.
    ///
    /// Nothing we build legitimately reaches this bound. It exists because `len as i32` wraps, and
    /// a wrapped prefix produces a frame the peer will misparse in a way neither side can explain.
    pub fn length(&mut self, value: usize) -> Result<()> {
        if value > self.limits.max_frame_len {
            return Err(InternalError::OversizedFrame {
                packet: self.packet,
                length: value,
                limit: self.limits.max_frame_len,
            }
            .into());
        }
        // Bounded by `max_frame_len`, so the cast is lossless.
        self.var_int(value as i32);
        Ok(())
    }

    /// Writes a length-prefixed byte slice.
    pub fn bytes(&mut self, value: &[u8]) -> Result<()> {
        self.length(value.len())?;
        self.buf.put_slice(value);
        Ok(())
    }

    /// Writes a length-prefixed string.
    pub fn string(&mut self, value: &str) -> Result<()> {
        self.bytes(value.as_bytes())
    }

    /// Writes a value preceded by a boolean saying whether it is there.
    ///
    /// The write half of [`Reader::optional`]. The boolean is emitted either way, so a `None` is
    /// one byte on the wire rather than nothing -- which is exactly what distinguishes this from a
    /// version-gated field, where a missing value costs no bytes at all.
    ///
    /// ```ignore
    /// w.optional(self.payload.as_deref(), |w, payload| w.bytes(payload))?;
    /// ```
    pub fn optional<T: ?Sized>(
        &mut self,
        value: Option<&T>,
        write: impl FnOnce(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        self.bool(value.is_some());
        match value {
            Some(value) => write(self, value),
            None => Ok(()),
        }
    }

    /// Writes a length-prefixed array of composite values.
    pub fn array<T: Wire>(&mut self, values: &[T], version: ProtocolVersion) -> Result<()> {
        self.length(values.len())?;
        for value in values {
            value.write(self, version)?;
        }
        Ok(())
    }

    /// Writes raw bytes without a length prefix.
    pub fn raw(&mut self, value: &[u8]) {
        self.buf.put_slice(value);
    }
}

/// A composite value that appears in more than one packet.
///
/// Primitives are deliberately absent: the *call* picks the encoding (`r.var_int()`, `r.u16()`),
/// which is a choice a `Wire for i32` impl could not express anyway. Implement this only for
/// structs and enums that are shared between packets -- a profile property, a chat component, a
/// known-pack entry.
pub trait Wire: Sized {
    /// Decodes the value.
    fn read(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self>;

    /// Encodes the value.
    fn write(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    fn reader(bytes: &[u8]) -> Reader<'_> {
        Reader::new(bytes, Limits::default())
    }

    fn write(f: impl FnOnce(&mut Writer<'_>)) -> BytesMut {
        let mut buf = BytesMut::new();
        f(&mut Writer::new(&mut buf, "Test", Limits::default()));
        buf
    }

    #[test]
    fn var_int_roundtrips() {
        for value in [0, 1, 127, 128, 255, 2_097_151, i32::MAX, -1, i32::MIN] {
            let buf = write(|w| w.var_int(value));
            assert_eq!(reader(&buf).var_int().expect("decodes"), value, "{value}");
        }
    }

    #[test]
    fn var_long_roundtrips_ten_byte_values() {
        for value in [0, 1, i64::MAX, -1, i64::MIN] {
            let buf = write(|w| w.var_long(value));
            let decoded = reader(&buf).var_long().expect("decodes");
            assert_eq!(decoded, value, "{value} encoded as {} bytes", buf.len());
        }
        // -1 needs all ten bytes; a nine byte bound silently truncates it.
        assert_eq!(write(|w| w.var_long(-1)).len(), 10);
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
        let err = r.string("server_address", 255).expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::NegativeLength { value: -1, .. })
        ));
    }

    #[test]
    fn huge_length_is_rejected_before_allocating() {
        // A ten byte packet claiming a 2 GiB string.
        let mut buf = write(|w| w.var_int(i32::MAX));
        buf.extend_from_slice(b"abcde");
        let err = reader(&buf)
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::LengthLimit { .. })
        ));
    }

    #[test]
    fn length_beyond_the_buffer_is_eof_not_an_allocation() {
        let mut buf = write(|w| w.var_int(4096));
        buf.extend_from_slice(b"abc");
        let err = reader(&buf)
            .bytes("payload", usize::MAX)
            .expect_err("must reject");
        assert!(matches!(err, Error::Protocol(ProtocolError::Eof { .. })));
    }

    #[test]
    fn a_per_field_limit_is_tighter_than_the_frame() {
        // 300 bytes fits a frame several times over, and is still refused: the field says 255.
        let host = "a".repeat(300);
        let buf = write(|w| w.string(&host).expect("fits a frame"));
        let err = reader(&buf)
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::LengthLimit { limit: 255, .. })
        ));
    }

    #[test]
    fn string_roundtrips() {
        let buf = write(|w| w.string("mc.justchunks.net").expect("writes"));
        assert_eq!(
            reader(&buf).string("host", 255).expect("decodes"),
            "mc.justchunks.net"
        );
    }

    #[test]
    fn an_unencodable_length_is_refused_instead_of_wrapping() {
        // The outbound counterpart of `FrameTooLarge`: no length prefix is emitted at all.
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(
            &mut buf,
            "Test",
            Limits {
                max_frame_len: 16,
                ..Limits::default()
            },
        );
        let err = writer.bytes(&[0u8; 32]).expect_err("must refuse");
        assert!(matches!(
            err,
            Error::Internal(InternalError::OversizedFrame { limit: 16, .. })
        ));
        assert!(buf.is_empty(), "nothing may reach the buffer");
    }

    #[test]
    fn fixed_width_numbers_roundtrip() {
        let buf = write(|w| {
            w.i8(i8::MIN);
            w.u8(u8::MAX);
            w.i16(i16::MIN);
            w.u16(u16::MAX);
            w.i32(i32::MIN);
            w.i64(i64::MIN);
            w.u64(u64::MAX);
            w.f32(std::f32::consts::PI);
            w.f64(-0.0);
        });
        let mut r = reader(&buf);
        assert_eq!(r.i8().expect("decodes"), i8::MIN);
        assert_eq!(r.u8().expect("decodes"), u8::MAX);
        assert_eq!(r.i16().expect("decodes"), i16::MIN);
        assert_eq!(r.u16().expect("decodes"), u16::MAX);
        assert_eq!(r.i32().expect("decodes"), i32::MIN);
        assert_eq!(r.i64().expect("decodes"), i64::MIN);
        assert_eq!(r.u64().expect("decodes"), u64::MAX);
        assert_eq!(r.f32().expect("decodes"), std::f32::consts::PI);
        // Big-endian, not native: `-0.0` differs from `0.0` only in the first byte.
        assert!(r.f64().expect("decodes").is_sign_negative());
        r.finish("Test").expect("consumes the whole payload");
    }

    #[test]
    fn a_fixed_width_number_that_runs_off_the_end_is_eof() {
        // Three bytes where four are needed: no panic, no partial value.
        let err = reader(&[0x01, 0x02, 0x03]).i32().expect_err("must reject");
        assert!(matches!(
            err,
            Error::Protocol(ProtocolError::Eof {
                needed: 4,
                remaining: 3
            })
        ));
    }

    #[test]
    fn an_optional_costs_a_byte_even_when_it_is_absent() {
        // The protocol's `Optional`, which is the peer's choice -- not `gated`, which is the
        // version's and reads nothing at all.
        let present = write(|w| {
            w.optional(Some("abc"), |w, value| w.string(value))
                .expect("writes");
        });
        let absent = write(|w| {
            w.optional(None::<&str>, |w, value| w.string(value))
                .expect("writes");
        });
        assert_eq!(absent.as_ref(), &[0x00]);

        let decoded = reader(&present)
            .optional(|r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded.as_deref(), Some("abc"));
        let decoded = reader(&absent)
            .optional(|r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded, None);
    }

    #[test]
    fn a_status_response_with_a_favicon_fits_the_default_frame() {
        // The regression the 8 KiB default caused: a 64x64 favicon is base64 of a PNG that runs to
        // several kilobytes, and refusing it would have been our own `OversizedFrame` for content
        // the client asks for by default.
        let favicon = "A".repeat(12 * 1024);
        let body = format!(r#"{{"favicon":"data:image/png;base64,{favicon}"}}"#);
        let mut buf = BytesMut::new();
        Writer::new(&mut buf, "StatusResponse", Limits::default())
            .string(&body)
            .expect("a favicon is ordinary content, not an oversized frame");
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
