use crate::wire::error::{Result, WireError};
use crate::wire::options::Options;
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
            }
            .into());
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
                return Err(WireError::VarIntTooLong { field, kind: KIND }.into());
            }
            result |= bits << (7 * index);
            if byte & 0b1000_0000 == 0 {
                if self.options.strict_varints && index > 0 && bits == 0 {
                    return Err(WireError::VarIntNotCanonical { field, kind: KIND }.into());
                }
                return Ok(result);
            }
        }
        Err(WireError::VarIntTooLong { field, kind: KIND }.into())
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
                return Err(WireError::VarIntTooLong { field, kind: KIND }.into());
            }
            result |= bits << (7 * index);
            if byte & 0b1000_0000 == 0 {
                if self.options.strict_varints && index > 0 && bits == 0 {
                    return Err(WireError::VarIntNotCanonical { field, kind: KIND }.into());
                }
                return Ok(result);
            }
        }
        Err(WireError::VarIntTooLong { field, kind: KIND }.into())
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
            return Err(WireError::NegativeLength { field, value: raw }.into());
        }
        // `raw` is non-negative, so the cast is lossless on every target we support.
        let length = raw as usize;
        if length > limit {
            return Err(WireError::LengthLimit {
                field,
                limit,
                actual: length,
            }
            .into());
        }
        let remaining = self.remaining();
        if length > remaining {
            return Err(WireError::Eof {
                field,
                needed: length,
                remaining,
            }
            .into());
        }
        Ok(length)
    }

    /// Reads a length-prefixed byte slice, borrowed from the underlying buffer.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining.
    pub fn bytes(&mut self, field: &'static str, limit: usize) -> Result<&'a [u8]> {
        let length = self.length(field, limit)?;
        self.take(field, length)
    }

    /// Reads a length-prefixed UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::Eof`] in case the reader has not enough bytes remaining. Additionally,
    /// it returns a [`WireError::Utf8`] if the string is an invalid UTF-8 encoding.
    pub fn string(&mut self, field: &'static str, limit: usize) -> Result<String> {
        let bytes = self.bytes(field, limit)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::Utf8 { field }.into())
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
    pub fn finish(&self, packet: &'static str) -> Result<()> {
        let remaining = self.remaining();
        if remaining > 0 {
            return Err(WireError::TrailingBytes { packet, remaining }.into());
        }
        Ok(())
    }
}
