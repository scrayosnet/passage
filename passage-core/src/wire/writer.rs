use crate::wire::error::{Result, WireError};
use crate::wire::options::Options;
use bytes::{BufMut, BytesMut};
use uuid::Uuid;

/// A writer for the Minecraft wire format.
pub struct Writer<'a> {
    buf: &'a mut BytesMut,
    options: Options,
}

impl<'a> Writer<'a> {
    /// Creates a writer that appends to `buf` with default options.
    #[must_use]
    pub fn new(buf: &'a mut BytesMut) -> Self {
        Self {
            buf,
            options: Options::default(),
        }
    }

    /// Overwrites the current [`Options`] with `options`, returning the updated reader.
    #[must_use]
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
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
    /// # Errors
    ///
    /// Returns an [`WireError::LengthLimit`] in case the length exceeds the limit.
    pub fn length(&mut self, field: &'static str, value: usize) -> Result<()> {
        if value > self.options.max_frame_len {
            return Err(WireError::LengthLimit {
                field,
                actual: value,
                limit: self.options.max_frame_len,
            });
        }
        // Bounded by `max_frame_len`, so the cast is lossless.
        self.var_int(value as i32);
        Ok(())
    }

    /// Writes a length-prefixed byte slice.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::LengthLimit`] in case the length exceeds the limit.
    pub fn bytes(&mut self, field: &'static str, value: &[u8]) -> Result<()> {
        self.length(field, value.len())?;
        self.buf.put_slice(value);
        Ok(())
    }

    // TODO use bytestring instead
    /// Writes a length-prefixed string.
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::LengthLimit`] in case the length exceeds the limit.
    pub fn string(&mut self, field: &'static str, value: &str) -> Result<()> {
        self.bytes(field, value.as_bytes())
    }

    /// Writes a value preceded by a boolean saying whether it is there. This is the write half of
    /// [`Reader::optional`](crate::wire::Reader::optional): the boolean is emitted either way, so a
    /// `None` costs one byte on the wire rather than nothing.
    ///
    /// ```ignore
    /// w.optional(self.payload.as_deref(), |w, payload| w.bytes("payload", payload))?;
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::LengthLimit`] in case the length exceeds the limit.
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
    ///
    /// # Errors
    ///
    /// Returns an [`WireError::LengthLimit`] in case the length exceeds the limit.
    pub fn array<T>(
        &mut self,
        field: &'static str,
        values: &[T],
        write: impl Fn(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        self.length(field, values.len())?;
        for value in values {
            write(self, value)?;
        }
        Ok(())
    }

    /// Writes raw bytes without a length prefix.
    pub fn raw(&mut self, value: &[u8]) {
        self.buf.put_slice(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(f: impl FnOnce(&mut Writer<'_>)) -> BytesMut {
        let mut buf = BytesMut::new();
        f(&mut Writer::new(&mut buf));
        buf
    }

    #[test]
    fn var_ints_are_encoded_minimally() {
        // The canonical form the reader insists on by default, from the side that produces it.
        assert_eq!(write(|w| w.var_int(0)).as_ref(), &[0x00]);
        assert_eq!(write(|w| w.var_int(127)).as_ref(), &[0x7F]);
        assert_eq!(write(|w| w.var_int(128)).as_ref(), &[0x80, 0x01]);
        assert_eq!(
            write(|w| w.var_int(-1)).as_ref(),
            &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]
        );
        assert_eq!(write(|w| w.var_int(i32::MAX)).len(), 5);
        assert_eq!(write(|w| w.var_long(i64::MIN)).len(), 10);
    }

    #[test]
    fn an_unencodable_length_is_refused_instead_of_wrapping() {
        // The outbound counterpart of an oversized frame: no length prefix is emitted at all, so a
        // prefix can never disagree with the payload that follows it.
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf).with_options(Options {
            max_frame_len: 16,
            ..Options::default()
        });
        let err = writer
            .bytes("payload", &[0u8; 32])
            .expect_err("must refuse");
        assert!(
            matches!(
                err,
                WireError::LengthLimit {
                    limit: 16,
                    actual: 32,
                    ..
                }
            ),
            "{err}"
        );
        assert!(buf.is_empty(), "nothing may reach the buffer");
    }

    #[test]
    fn an_array_that_does_not_fit_is_refused_before_its_elements() {
        let mut buf = BytesMut::new();
        let values = vec![0u8; 32];
        let err = Writer::new(&mut buf)
            .with_options(Options {
                max_frame_len: 4,
                ..Options::default()
            })
            .array("values", &values, |w, value| {
                w.u8(*value);
                Ok(())
            })
            .expect_err("must refuse");
        assert!(
            matches!(err, WireError::LengthLimit { limit: 4, .. }),
            "{err}"
        );
        assert!(buf.is_empty(), "nothing may reach the buffer");
    }

    #[test]
    fn a_status_response_with_a_favicon_fits_the_default_frame() {
        // The regression an 8 KiB default caused: a 64x64 favicon is base64 of a PNG that runs to
        // several kilobytes, and refusing it would have been our own error for content the client
        // asks for by default.
        let favicon = "A".repeat(12 * 1024);
        let body = format!(r#"{{"favicon":"data:image/png;base64,{favicon}"}}"#);
        let mut buf = BytesMut::new();
        Writer::new(&mut buf)
            .string("body", &body)
            .expect("a favicon is ordinary content, not an oversized frame");
    }

    #[test]
    fn the_writer_reports_what_it_has_written() {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        assert!(writer.is_empty());
        writer.u8(0x01);
        assert_eq!(writer.len(), 1);
        assert!(!writer.is_empty());
    }

    #[test]
    fn raw_bytes_carry_no_prefix() {
        // What the frame codec writes a payload with: the length was already accounted for.
        assert_eq!(write(|w| w.raw(&[0x01, 0x02])).as_ref(), &[0x01, 0x02]);
    }
}
