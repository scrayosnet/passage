use crate::ProtocolVersion;
use crate::wire::error::{Result, WireError};
use crate::wire::options::Options;
use crate::wire::property::Property;
use uuid::Uuid;

/// A bounds-checked reader over a packet payload.
///
/// ```ignored
/// let mut r = Reader::new(buf).with_options(Options::permissive());
/// ```
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    options: Options,
}

impl<'a> Reader<'a> {
    /// Creates a new reader over `buf` with default options.
    #[must_use]
    pub fn new(buf: &'a [u8]) -> Self {
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

    /// Consumes exactly `n` bytes and returns them. The resulting slice is borrowed from the
    /// underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has less than `n` bytes remaining.
    pub fn take(&mut self, field: &'static str, n: usize) -> Result<&'a [u8]> {
        let remaining = self.remaining();
        if remaining < n {
            return Err(WireError::Eof {
                field,
                needed: n,
                remaining,
            });
        }
        let slice = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    /// Consumes exactly `N` bytes as an array. The resulting array is copied from the underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn fixed<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N]> {
        let mut array = [0u8; N];
        array.copy_from_slice(self.take(field, N)?);
        Ok(array)
    }

    /// Reads a single byte.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn u8(&mut self, field: &'static str) -> Result<u8> {
        Ok(self.take(field, 1)?[0])
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
        const KIND: &str = "VarInt";
        let mut result: i32 = 0;
        for index in 0..5 {
            let byte = self.u8(field)?;
            let bits = i32::from(byte & 0b0111_1111);
            // The fifth byte only has four significant bits; anything else would silently wrap.
            if index == 4 && bits > 0b1111 {
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

    /// Reads a length-prefixed byte slice, borrowed from the underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns whatever [`length`](Reader::length) raises for the prefix, and an
    /// [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn bytes(&mut self, field: &'static str, limit: usize) -> Result<&'a [u8]> {
        let length = self.length(field, limit)?;
        self.take(field, length)
    }

    /// Reads a length-prefixed UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns whatever [`bytes`](Reader::bytes) raises, and a [`WireError::Utf8`] if what it read is
    /// not valid UTF-8.
    pub fn string(&mut self, field: &'static str, limit: usize) -> Result<String> {
        let bytes = self.bytes(field, limit)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::Utf8 { field })
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
    /// session_id: r.gated(version.at_least(versions::V26_2), Reader::uuid)?,
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

    /// Borrows the rest of the buffer without consuming it.
    #[must_use]
    pub fn peek_rest(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    /// Consumes the rest of the buffer.
    pub fn pop_rest(&mut self) -> &'a [u8] {
        let slice = &self.buf[self.pos..];
        self.pos = self.buf.len();
        slice
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
            let decoded = Reader::new(&buf).var_int("value").expect("decodes");
            assert_eq!(decoded, value, "{value}");
        }
    }

    #[test]
    fn var_long_roundtrips_ten_byte_values() {
        for value in [0, 1, i64::MAX, -1, i64::MIN] {
            let buf = write(|w| w.var_long(value));
            let decoded = Reader::new(&buf).var_long("value").expect("decodes");
            assert_eq!(decoded, value, "{value} encoded as {} bytes", buf.len());
        }
        // -1 needs all ten bytes; a nine byte bound silently truncates it.
        assert_eq!(write(|w| w.var_long(-1)).len(), 10);
    }

    #[test]
    fn var_int_rejects_overlong_encodings() {
        // Six continuation bytes.
        let err = Reader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x01])
            .var_int("value")
            .expect_err("must reject");
        assert!(
            matches!(err, WireError::VarIntTooLong { kind: "VarInt", .. }),
            "{err}"
        );

        // Five bytes whose last byte overflows 32 bits.
        let err = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF])
            .var_int("value")
            .expect_err("must reject");
        assert!(matches!(err, WireError::VarIntTooLong { .. }), "{err}");

        // And the same for the ten-byte form.
        let err = Reader::new(&[0xFF; 10])
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
        let err = Reader::new(&[0x80, 0x00])
            .var_int("value")
            .expect_err("must reject");
        assert!(matches!(err, WireError::VarIntNotCanonical { .. }), "{err}");

        let decoded = Reader::new(&[0x80, 0x00])
            .with_options(Options::permissive())
            .var_int("value")
            .expect("permissive options accept it");
        assert_eq!(decoded, 0);
    }

    #[test]
    fn a_field_names_itself_in_every_error() {
        // The single best thing a wire error carries: *which* field disagreed, not only that one
        // did. Every path that can fail has to keep it.
        let err = Reader::new(&[]).u8("first").expect_err("eof");
        assert!(err.to_string().contains("first"), "{err}");
        let err = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F])
            .string("second", 16)
            .expect_err("negative length");
        assert!(err.to_string().contains("second"), "{err}");
    }

    #[test]
    fn negative_length_is_rejected_before_allocating() {
        // 0xFF 0xFF 0xFF 0xFF 0x0F decodes to -1. Cast to `usize` this is 18446744073709551615.
        let err = Reader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F])
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
        let err = Reader::new(&buf)
            .string("server_address", 255)
            .expect_err("must reject");
        assert!(matches!(err, WireError::LengthLimit { .. }), "{err}");
    }

    #[test]
    fn length_beyond_the_buffer_is_eof_not_an_allocation() {
        let mut buf = write(|w| w.var_int(4096));
        buf.extend_from_slice(b"abc");
        let err = Reader::new(&buf)
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
        let err = Reader::new(&buf)
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
            Reader::new(&buf).string("host", 255).expect("decodes"),
            "mc.justchunks.net"
        );

        let buf = write(|w| w.bytes("host", &[0xFF, 0xFE]).expect("writes"));
        let err = Reader::new(&buf)
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
        let mut r = Reader::new(&buf);
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
        let err = Reader::new(&[0x01, 0x02, 0x03])
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

        let decoded = Reader::new(&present)
            .optional("value", |r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded.as_deref(), Some("abc"));
        let decoded = Reader::new(&absent)
            .optional("value", |r| r.string("value", 16))
            .expect("decodes");
        assert_eq!(decoded, None);
    }

    #[test]
    fn a_gated_field_reads_nothing_at_all_when_it_is_not_there() {
        // The version's choice, not the peer's: an old client's payload has no byte for it.
        let buf = write(|w| w.u8(0x07));
        let mut r = Reader::new(&buf);
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
        let names = Reader::new(&buf)
            .array("names", 8, |r| r.string("name", 16))
            .expect("decodes");
        assert_eq!(names, vec!["a".to_owned(), "bb".to_owned()]);

        // The element limit is the allocation bound, so it is checked before any element is read.
        let err = Reader::new(&buf)
            .array("names", 1, |r| r.string("name", 16))
            .expect_err("must reject");
        assert!(
            matches!(err, WireError::LengthLimit { limit: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn trailing_bytes_are_reported() {
        let mut r = Reader::new(&[0x01, 0x02]);
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
        let mut r = Reader::new(&[0x01, 0x02, 0x03]);
        assert_eq!((r.position(), r.remaining()), (0, 3));
        r.u8("first").expect("reads");
        assert_eq!((r.position(), r.remaining()), (1, 2));
        assert_eq!(r.peek_rest(), &[0x02, 0x03]);
        assert_eq!(r.remaining(), 2, "peeking does not consume");
        assert_eq!(r.pop_rest(), &[0x02, 0x03]);
        assert_eq!((r.position(), r.remaining()), (3, 0));
        r.finish("Demo").expect("nothing is left");
    }

    #[test]
    fn take_and_fixed_are_available_to_a_codec_the_crate_knows_nothing_about() {
        let mut r = Reader::new(&[0x01, 0x02, 0x03, 0x04]);
        assert_eq!(r.take("prefix", 1).expect("reads"), &[0x01]);
        assert_eq!(r.fixed::<2>("middle").expect("reads"), [0x02, 0x03]);
        assert!(r.fixed::<2>("rest").is_err(), "one byte is left, not two");
    }
}
