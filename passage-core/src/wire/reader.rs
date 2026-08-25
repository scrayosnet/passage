use crate::ProtocolVersion;
use crate::wire::error::{Result, WireError};
use crate::wire::options::Options;
use crate::wire::property::Property;
use bytes::Bytes;
use bytestring::ByteString;
use uuid::Uuid;

/// The most bytes a `VarInt` can occupy.
const MAX_VAR_INT_LEN: usize = 5;

/// Reads a `VarInt` from the front of `buf`, returning it and how many bytes it took.
///
/// This is [`Reader::var_int`] without the cursor, for framing code that has no frame yet: the
/// length prefix is read out of the socket's buffer, which cannot be sliced or owned until the
/// frame it announces has arrived in full. It is the only read with that problem, and the only one
/// with a free function -- everything else happens after [`FrameCodec`](crate::codec::FrameCodec)
/// has split a frame off.
///
/// # Errors
///
/// Returns an [`WireError::Eof`] if `buf` ends mid-value -- which is backpressure, not a malformed
/// frame -- and a [`WireError::VarIntTooLong`] if the value does not fit an `i32`. Additionally, if
/// strict varints are enabled, a [`WireError::VarIntNotCanonical`] is returned if the value is
/// non-canonical (overlong).
pub fn read_var_int(buf: &[u8], field: &'static str, options: Options) -> Result<(i32, usize)> {
    const KIND: &str = "VarInt";
    let mut result: i32 = 0;
    for index in 0..MAX_VAR_INT_LEN {
        let byte = *buf.get(index).ok_or(WireError::Eof {
            field,
            needed: index + 1,
            remaining: buf.len(),
        })?;
        let bits = i32::from(byte & 0b0111_1111);
        // The fifth byte only has four significant bits; anything else would silently wrap.
        if index == MAX_VAR_INT_LEN - 1 && bits > 0b1111 {
            return Err(WireError::VarIntTooLong { field, kind: KIND });
        }
        result |= bits << (7 * index);
        if byte & 0b1000_0000 == 0 {
            if options.strict_varints && index > 0 && bits == 0 {
                return Err(WireError::VarIntNotCanonical { field, kind: KIND });
            }
            return Ok((result, index + 1));
        }
    }
    Err(WireError::VarIntTooLong { field, kind: KIND })
}

/// A bounds-checked reader over a packet payload.
///
/// ```ignored
/// let mut r = Reader::new(frame.payload).with_options(Options::permissive());
/// ```
///
/// The reader owns the payload, which is the [`Bytes`] a [`Frame`](crate::codec::Frame) was split
/// into. A field the packet keeps whole -- every string, a cookie's payload, a plugin message's
/// data -- is a slice of that buffer rather than a copy of it, so decoding a packet allocates only
/// for what it reshapes. What the packet keeps holds the frame alive; everything else is read
/// through a plain `&[u8]` view and costs nothing.
pub struct Reader {
    buf: Bytes,
    pos: usize,
    options: Options,
}

impl Reader {
    /// Creates a new reader over `buf` with default options.
    #[must_use]
    pub fn new(buf: Bytes) -> Self {
        Self {
            buf,
            pos: 0,
            options: Options::default(),
        }
    }

    /// Overwrites the current [`Options`] with `options`, returning the updated reader.
    #[must_use]
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Gets the configured reader options.
    #[must_use]
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Gets the number of bytes consumed so far.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Gets the number of bytes left in the buffer.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Consumes exactly `n` bytes and returns them as a [`Bytes`], which shares the payload's
    /// allocation rather than copying out of it.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has less than `n` bytes remaining.
    pub fn take(&mut self, field: &'static str, n: usize) -> Result<Bytes> {
        let start = self.advance(field, n)?;
        Ok(self.buf.slice(start..start + n))
    }

    /// Consumes exactly `n` bytes without reading them, for a field that was decoded by walking it
    /// through [`peek_rest`](Reader::peek_rest).
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has less than `n` bytes remaining.
    pub fn skip(&mut self, field: &'static str, n: usize) -> Result<()> {
        self.advance(field, n)?;
        Ok(())
    }

    /// Consumes exactly `n` bytes and views them, without touching the refcount.
    ///
    /// This is what the fixed-width reads are built on: an `i32` is copied out of the buffer either
    /// way, so paying for a slice of it would be a refcount bump for nothing.
    fn view(&mut self, field: &'static str, n: usize) -> Result<&[u8]> {
        let start = self.advance(field, n)?;
        Ok(&self.buf[start..start + n])
    }

    /// Consumes `n` bytes, returning where they start.
    ///
    /// The one place the cursor moves, and the one place the bound is checked.
    fn advance(&mut self, field: &'static str, n: usize) -> Result<usize> {
        let remaining = self.remaining();
        if remaining < n {
            return Err(WireError::Eof {
                field,
                needed: n,
                remaining,
            });
        }
        let start = self.pos;
        self.pos += n;
        Ok(start)
    }

    /// Consumes exactly `N` bytes as an array. The resulting array is copied from the underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn fixed<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N]> {
        let mut array = [0u8; N];
        array.copy_from_slice(self.view(field, N)?);
        Ok(array)
    }

    /// Reads a single byte.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn u8(&mut self, field: &'static str) -> Result<u8> {
        Ok(self.view(field, 1)?[0])
    }

    /// Reads a signed byte.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn i8(&mut self, field: &'static str) -> Result<i8> {
        Ok(self.u8(field)? as i8)
    }

    /// Reads a boolean. Any non-zero byte is `true`, matching the vanilla implementation.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn bool(&mut self, field: &'static str) -> Result<bool> {
        Ok(self.u8(field)? != 0)
    }

    /// Reads a big-endian `u16`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn u16(&mut self, field: &'static str) -> Result<u16> {
        Ok(u16::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `i16`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn i16(&mut self, field: &'static str) -> Result<i16> {
        Ok(i16::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `i32`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn i32(&mut self, field: &'static str) -> Result<i32> {
        Ok(i32::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `i64`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn i64(&mut self, field: &'static str) -> Result<i64> {
        Ok(i64::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `u64`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn u64(&mut self, field: &'static str) -> Result<u64> {
        Ok(u64::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `f32`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn f32(&mut self, field: &'static str) -> Result<f32> {
        Ok(f32::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a big-endian `f64`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn f64(&mut self, field: &'static str) -> Result<f64> {
        Ok(f64::from_be_bytes(self.fixed(field)?))
    }

    /// Reads a UUID (two big-endian `u64`s).
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn uuid(&mut self, field: &'static str) -> Result<Uuid> {
        Ok(Uuid::from_bytes(self.fixed(field)?))
    }

    /// Reads a `VarInt`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining and a
    /// [`WireError::VarIntTooLong`] in case the value is too long (invalid encoding). Additionally, if
    /// strict varints are enabled, a [`WireError::VarIntNotCanonical`] is returned if the value is
    /// non-canonical (overlong).
    pub fn var_int(&mut self, field: &'static str) -> Result<i32> {
        let (value, length) = read_var_int(self.peek_rest(), field, self.options)?;
        self.pos += length;
        Ok(value)
    }

    /// Reads a `VarLong`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining and a
    /// [`WireError::VarIntTooLong`] in case the value is too long (invalid encoding). Additionally, if
    /// strict varints are enabled, a [`WireError::VarIntNotCanonical`] is returned if the value is
    /// non-canonical (overlong).
    pub fn var_long(&mut self, field: &'static str) -> Result<i64> {
        const KIND: &str = "VarLong";
        let mut result: i64 = 0;
        for index in 0..10 {
            let byte = self.u8(field)?;
            let bits = i64::from(byte & 0b0111_1111);
            // The tenth byte only has one significant bit.
            if index == 9 && bits > 0b1 {
                return Err(WireError::VarIntTooLong { field, kind: KIND });
            }
            result |= bits << (7 * index);
            if byte & 0b1000_0000 == 0 {
                if self.options.strict_varints && index > 0 && bits == 0 {
                    return Err(WireError::VarIntNotCanonical { field, kind: KIND });
                }
                return Ok(result);
            }
        }
        Err(WireError::VarIntTooLong { field, kind: KIND })
    }

    /// Reads a length prefix for `field`.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining. Additionally,
    /// if the raw length is negative or exceeds the configured limits, it returns a [`WireError::NegativeLength`]
    /// or [`WireError::LengthLimit`] respectively.
    pub fn length(&mut self, field: &'static str, limit: usize) -> Result<usize> {
        let raw = self.var_int(field)?;
        if raw < 0 {
            return Err(WireError::NegativeLength { field, value: raw });
        }
        // `raw` is non-negative, so the cast is lossless on every target we support.
        let length = raw as usize;
        if length > limit {
            return Err(WireError::LengthLimit {
                field,
                limit,
                actual: length,
            });
        }
        let remaining = self.remaining();
        if length > remaining {
            return Err(WireError::Eof {
                field,
                needed: length,
                remaining,
            });
        }
        Ok(length)
    }

    /// Reads a length-prefixed byte slice, cut from the underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns whatever [`length`](Reader::length) raises for the prefix, and an
    /// [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn bytes(&mut self, field: &'static str, limit: usize) -> Result<Bytes> {
        let length = self.length(field, limit)?;
        self.take(field, length)
    }

    /// Reads a length-prefixed UTF-8 string, cut from the underlying buffer.
    ///
    /// The bytes are validated, not copied: a [`ByteString`] is a [`Bytes`] that is known to be
    /// UTF-8, so a string field costs the same as the byte field underneath it.
    ///
    /// # Errors
    ///
    /// Returns whatever [`bytes`](Reader::bytes) raises, and a [`WireError::Utf8`] if what it read is
    /// not valid UTF-8.
    pub fn string(&mut self, field: &'static str, limit: usize) -> Result<ByteString> {
        let bytes = self.bytes(field, limit)?;
        ByteString::try_from(bytes).map_err(|_| WireError::Utf8 { field })
    }

    /// Reads a length-prefixed array of composite values with up to `limit` elements.
    ///
    /// ```ignore
    /// properties: r.array("properties", 10, |r| r.string("property", 10))?,
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining. Additionally,
    /// if the raw length is negative or exceeds the limit, it returns a [`WireError::NegativeLength`]
    /// or [`WireError::LengthLimit`] respectively. Nested read errors are passed on.
    pub fn array<T>(
        &mut self,
        field: &'static str,
        limit: usize,
        read: impl Fn(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let count = self.length(field, limit)?;
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(read(self)?);
        }
        Ok(values)
    }

    /// Reads a value that knows how to read itself, naming it `field` in whatever it fails with.
    ///
    /// The version is handed on because a [`Property`] may be encoded differently in different
    /// versions, and the value is the only thing that knows whether it is.
    pub fn property<T: Property>(
        &mut self,
        version: ProtocolVersion,
        field: &'static str,
    ) -> Result<T> {
        T::decode(self, version, field)
    }

    /// Reads a value that only exists once `condition` holds, and [`None`] otherwise. This is
    /// different from [`optional`](Reader::optional), which reads a boolean and then an optional value.
    ///
    /// ```ignore
    /// session_id: r.gated(version.at_least(versions::V26_1), Reader::uuid)?,
    /// ```
    ///
    /// # Errors
    ///
    /// Returns any nested read errors. Infallible if `condition` is `false`.
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

    /// Reads a value preceded by a boolean saying whether it is there. This is different from
    /// [`gated`](Reader::gated), which only reads if the `condition` is `true`.
    ///
    /// ```ignore
    /// payload: r.optional(|r| r.bytes("payload", MAX_COOKIE_LEN))?,
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining. Nested read
    /// errors are passed on.
    pub fn optional<T>(
        &mut self,
        field: &'static str,
        read: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<Option<T>> {
        let present = self.bool(field)?;
        self.gated(present, read)
    }

    /// Views the rest of the buffer without consuming it.
    #[must_use]
    pub fn peek_rest(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    /// Consumes the rest of the buffer, which is what a field with no length of its own -- a plugin
    /// message's data -- is made of.
    pub fn pop_rest(&mut self) -> Bytes {
        let rest = self.buf.slice(self.pos..);
        self.pos = self.buf.len();
        rest
    }

    /// Asserts that the whole payload was consumed.
    ///
    /// # Errors
    ///
    /// Returns a [`WireError::TrailingBytes`] if any byte of the payload was left undecoded.
    pub fn finish(&self, packet: &'static str) -> Result<()> {
        let remaining = self.remaining();
        if remaining > 0 {
            return Err(WireError::TrailingBytes { packet, remaining });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;
    use bytes::BytesMut;

    /// Encodes with the writer, so a round trip is asserted against the encoder it will meet.
    fn write(f: impl FnOnce(&mut Writer<'_>)) -> BytesMut {
        let mut buf = BytesMut::new();
        f(&mut Writer::new(&mut buf));
        buf
    }

    #[test]
    fn var_int_roundtrips() {
        for value in [0, 1, 127, 128, 255, 2_097_151, i32::MAX, -1, i32::MIN] {
            let buf = write(|w| w.var_int(value));
            let decoded = Reader::new(buf.clone().freeze())
                .var_int("value")
                .expect("decodes");
            assert_eq!(decoded, value, "{value}");
        }
    }

    #[test]
    fn var_long_roundtrips_ten_byte_values() {
        for value in [0, 1, i64::MAX, -1, i64::MIN] {
            let buf = write(|w| w.var_long(value));
            let decoded = Reader::new(buf.clone().freeze())
                .var_long("value")
                .expect("decodes");
            assert_eq!(decoded, value, "{value} encoded as {} bytes", buf.len());
        }
        // -1 needs all ten bytes; a nine byte bound silently truncates it.
        assert_eq!(write(|w| w.var_long(-1)).len(), 10);
    }

    #[test]
    fn var_int_rejects_overlong_encodings() {
        // Six continuation bytes.
        let err = Reader::new(Bytes::from_static(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01]))
            .var_int("value")
            .expect_err("must reject");
        assert!(
            matches!(err, WireError::VarIntTooLong { kind: "VarInt", .. }),
            "{err}"
        );

        // Five bytes whose last byte overflows 32 bits.
        let err = Reader::new(Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]))
            .var_int("value")
            .expect_err("must reject");
        assert!(matches!(err, WireError::VarIntTooLong { .. }), "{err}");

        // And the same for the ten-byte form.
        let err = Reader::new(Bytes::from_static(&[0xFF; 10]))
            .var_long("value")
            .expect_err("must reject");
        assert!(
            matches!(
                err,
                WireError::VarIntTooLong {
                    kind: "VarLong",
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_non_canonical_var_int_is_refused_only_when_asked_to_be() {
        // The same value encoded two ways is a parser-differential surface, but the reference
        // implementation accepts it -- hence the switch rather than a rule.
        let err = Reader::new(Bytes::from_static(&[0x80, 0x00]))
            .var_int("value")
            .expect_err("must reject");
        assert!(matches!(err, WireError::VarIntNotCanonical { .. }), "{err}");

        let decoded = Reader::new(Bytes::from_static(&[0x80, 0x00]))
            .with_options(Options::permissive())
            .var_int("value")
            .expect("permissive options accept it");
        assert_eq!(decoded, 0);
    }

    #[test]
    fn a_field_names_itself_in_every_error() {
        // The single best thing a wire error carries: *which* field disagreed, not only that one
        // did. Every path that can fail has to keep it.
        let err = Reader::new(Bytes::from_static(&[]))
            .u8("first")
            .expect_err("eof");
        assert!(err.to_string().contains("first"), "{err}");
        let err = Reader::new(Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]))
            .string("second", 16)
            .expect_err("negative length");
        assert!(err.to_string().contains("second"), "{err}");
    }

    #[test]
    fn negative_length_is_rejected_before_allocating() {
        // 0xFF 0xFF 0xFF 0xFF 0x0F decodes to -1. Cast to `usize` this is 18446744073709551615.
        let err = Reader::new(Bytes::from_static(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]))
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(
            matches!(err, WireError::NegativeLength { value: -1, .. }),
            "{err}"
        );
    }

    #[test]
    fn huge_length_is_rejected_before_allocating() {
        // A ten byte packet claiming a 2 GiB string.
        let mut buf = write(|w| w.var_int(i32::MAX));
        buf.extend_from_slice(b"abcde");
        let err = Reader::new(buf.clone().freeze())
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(matches!(err, WireError::LengthLimit { .. }), "{err}");
    }

    #[test]
    fn length_beyond_the_buffer_is_eof_not_an_allocation() {
        let mut buf = write(|w| w.var_int(4096));
        buf.extend_from_slice(b"abc");
        let err = Reader::new(buf.clone().freeze())
            .bytes("payload", usize::MAX)
            .expect_err("must reject");
        assert!(
            matches!(
                err,
                WireError::Eof {
                    needed: 4096,
                    remaining: 3,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn a_per_field_limit_is_tighter_than_the_frame() {
        // 300 bytes fits a frame several times over, and is still refused: the field says 255.
        let host = "a".repeat(300);
        let buf = write(|w| w.string("host", &host).expect("fits a frame"));
        let err = Reader::new(buf.clone().freeze())
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(
            matches!(
                err,
                WireError::LengthLimit {
                    limit: 255,
                    actual: 300,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn string_roundtrips_and_invalid_utf8_is_named() {
        let buf = write(|w| w.string("host", "mc.justchunks.net").expect("writes"));
        assert_eq!(
            Reader::new(buf.clone().freeze())
                .string("host", 255)
                .expect("decodes"),
            "mc.justchunks.net"
        );

        let buf = write(|w| w.bytes("host", &[0xFF, 0xFE]).expect("writes"));
        let err = Reader::new(buf.clone().freeze())
            .string("host", 255)
            .expect_err("not UTF-8");
        assert!(matches!(err, WireError::Utf8 { field: "host" }), "{err}");
    }

    #[test]
    fn fixed_width_numbers_roundtrip() {
        let buf = write(|w| {
            w.i8(i8::MIN);
            w.u8(u8::MAX);
            w.bool(true);
            w.i16(i16::MIN);
            w.u16(u16::MAX);
            w.i32(i32::MIN);
            w.i64(i64::MIN);
            w.u64(u64::MAX);
            w.f32(std::f32::consts::PI);
            w.f64(-0.0);
            w.uuid(&uuid::Uuid::from_u128(1));
        });
        let mut r = Reader::new(buf.clone().freeze());
        assert_eq!(r.i8("i8").expect("decodes"), i8::MIN);
        assert_eq!(r.u8("u8").expect("decodes"), u8::MAX);
        assert!(r.bool("bool").expect("decodes"));
        assert_eq!(r.i16("i16").expect("decodes"), i16::MIN);
        assert_eq!(r.u16("u16").expect("decodes"), u16::MAX);
        assert_eq!(r.i32("i32").expect("decodes"), i32::MIN);
        assert_eq!(r.i64("i64").expect("decodes"), i64::MIN);
        assert_eq!(r.u64("u64").expect("decodes"), u64::MAX);
        assert_eq!(r.f32("f32").expect("decodes"), std::f32::consts::PI);
        // Big-endian, not native: `-0.0` differs from `0.0` only in the first byte.
        assert!(r.f64("f64").expect("decodes").is_sign_negative());
        assert_eq!(r.uuid("uuid").expect("decodes"), uuid::Uuid::from_u128(1));
        r.finish("Test").expect("consumes the whole payload");
    }

    #[test]
    fn a_fixed_width_number_that_runs_off_the_end_is_eof() {
        // Three bytes where four are needed: no panic, no partial value.
        let err = Reader::new(Bytes::from_static(&[0x01, 0x02, 0x03]))
            .i32("value")
            .expect_err("must reject");
        assert!(
            matches!(
                err,
                WireError::Eof {
                    needed: 4,
                    remaining: 3,
                    ..
                }
            ),
            "{err}"
        );
    }

    #[test]
    fn an_optional_costs_a_byte_even_when_it_is_absent() {
        // The protocol's `Optional`, which is the peer's choice -- not `gated`, which is the
        // version's and reads nothing at all.
        let present = write(|w| {
            w.optional(Some("abc"), |w, value| w.string("value", value))
                .expect("writes");
        });
        let absent = write(|w| {
            w.optional(None::<&str>, |w, value| w.string("value", value))
                .expect("writes");
        });
        assert_eq!(absent.as_ref(), &[0x00]);

        let decoded = Reader::new(present.clone().freeze())
            .optional("value", |r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded.as_deref(), Some("abc"));
        let decoded = Reader::new(absent.clone().freeze())
            .optional("value", |r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded, None);
    }

    #[test]
    fn a_gated_field_reads_nothing_at_all_when_it_is_not_there() {
        // The version's choice, not the peer's: an old client's payload has no byte for it.
        let buf = write(|w| w.u8(0x07));
        let mut r = Reader::new(buf.clone().freeze());
        assert_eq!(r.gated(false, |r| r.u8("gated")).expect("decodes"), None);
        assert_eq!(r.remaining(), 1, "a closed gate must not consume a byte");
        assert_eq!(
            r.gated(true, |r| r.u8("gated")).expect("decodes"),
            Some(0x07)
        );
    }

    #[test]
    fn an_array_reads_exactly_as_many_elements_as_it_promised() {
        let buf = write(|w| {
            w.array("names", &["a", "bb"], |w, value| w.string("name", value))
                .expect("writes");
        });
        let names = Reader::new(buf.clone().freeze())
            .array("names", 8, |r| r.string("name", 16))
            .expect("decodes");
        assert_eq!(names, vec!["a".to_owned(), "bb".to_owned()]);

        // The element limit is the allocation bound, so it is checked before any element is read.
        let err = Reader::new(buf.clone().freeze())
            .array("names", 1, |r| r.string("name", 16))
            .expect_err("must reject");
        assert!(
            matches!(err, WireError::LengthLimit { limit: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut r = Reader::new(Bytes::from_static(&[0x01, 0x02]));
        r.u8("first").expect("reads");
        assert!(
            matches!(
                r.finish("Demo").expect_err("must reject"),
                WireError::TrailingBytes { remaining: 1, .. }
            ),
            "one byte is left over",
        );
    }

    #[test]
    fn the_cursor_reports_where_it_is() {
        let mut r = Reader::new(Bytes::from_static(&[0x01, 0x02, 0x03]));
        assert_eq!((r.position(), r.remaining()), (0, 3));
        r.u8("first").expect("reads");
        assert_eq!((r.position(), r.remaining()), (1, 2));
        assert_eq!(r.peek_rest(), &[0x02, 0x03]);
        assert_eq!(r.remaining(), 2, "peeking does not consume");
        assert_eq!(r.pop_rest().as_ref(), &[0x02, 0x03]);
        assert_eq!((r.position(), r.remaining()), (3, 0));
        r.finish("Demo").expect("nothing is left");
    }

    #[test]
    fn a_field_the_packet_keeps_is_a_slice_of_the_payload_rather_than_a_copy() {
        // The reason the reader owns its buffer: a field that is kept whole is an offset into the
        // frame, so decoding a packet allocates nothing at all.
        let payload = Bytes::from_static(b"\x03abc\x03def\xFF\xFE");
        let mut r = Reader::new(payload.clone());
        let first = r.bytes("first", 16).expect("reads");
        let second = r.string("second", 16).expect("reads");
        let rest = r.pop_rest();

        assert_eq!(first.as_ref(), b"abc");
        assert_eq!(second, "def");
        assert_eq!(rest.as_ref(), &[0xFF, 0xFE]);
        assert_eq!(first.as_ptr(), payload[1..].as_ptr(), "no copy");
        assert_eq!(second.as_bytes().as_ptr(), payload[5..].as_ptr(), "no copy");
        assert_eq!(rest.as_ptr(), payload[8..].as_ptr(), "no copy");
    }

    #[test]
    fn an_empty_field_is_cut_without_a_panic() {
        // A slice of nothing at the end of the buffer: the one input that could turn a cookie the
        // client does not have into a crash.
        let mut r = Reader::new(Bytes::from_static(b"\x00\x00"));
        assert!(r.bytes("payload", 16).expect("reads").is_empty());
        assert!(r.string("name", 16).expect("reads").is_empty());
        assert!(r.pop_rest().is_empty());
        r.finish("Demo").expect("consumes the whole payload");
    }

    #[test]
    fn take_and_fixed_are_available_to_a_codec_the_crate_knows_nothing_about() {
        let mut r = Reader::new(Bytes::from_static(&[0x01, 0x02, 0x03, 0x04]));
        assert_eq!(r.take("prefix", 1).expect("reads").as_ref(), &[0x01]);
        assert_eq!(r.fixed::<2>("middle").expect("reads"), [0x02, 0x03]);
        assert!(r.fixed::<2>("rest").is_err(), "one byte is left, not two");
    }

    #[test]
    fn a_length_prefix_can_be_read_before_there_is_a_frame_to_own() {
        // What the codec does with the socket's buffer: the same varint the reader reads, minus the
        // cursor -- and a value that is not there yet is backpressure rather than a bad frame.
        let (value, length) =
            read_var_int(&[0x80, 0x01, 0xFF], "packet_length", Options::default()).expect("reads");
        assert_eq!((value, length), (128, 2));

        let err = read_var_int(&[0x80], "packet_length", Options::default()).expect_err("eof");
        assert!(
            matches!(
                err,
                WireError::Eof {
                    needed: 2,
                    remaining: 1,
                    ..
                }
            ),
            "{err}"
        );

        // And it agrees with the reader, because the reader is this function plus a cursor.
        let buf = write(|w| w.var_int(128));
        let mut r = Reader::new(buf.freeze());
        assert_eq!(r.var_int("value").expect("reads"), 128);
        assert_eq!(r.position(), 2);
    }
}
