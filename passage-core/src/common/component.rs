use crate::ProtocolVersion;
use crate::wire::{Property, Reader, WireError, WireResult, Writer};
use fastnbt::{DeOpts, SerOpts, Value};
use std::io::Cursor;

/// The tag byte of a `TAG_String`, which is the shape a plain text component takes on the wire.
const TAG_STRING: u8 = 0x08;

/// The maximum length of a `TAG_String` payload, which carries a `u16` length prefix.
const TAG_STRING_LIMIT: usize = u16::MAX as usize;

/// A network NBT value: a tag byte followed by the value, with no name on the root compound.
///
/// The value is read by walking it, so an NBT field does not have to be the last one in a packet --
/// registry entries and dialogs carry theirs in the middle of an array.
#[derive(Debug, Clone, PartialEq)]
pub struct Nbt(pub Value);

impl Property for Nbt {
    const NAME: &'static str = "nbt";

    fn decode(r: &mut Reader<'_>, _: ProtocolVersion, field: &'static str) -> WireResult<Self> {
        // The NBT codec reads exactly the bytes the value occupies and nothing beyond it, so the
        // cursor's position is how far the field reached into the payload.
        let mut cursor = Cursor::new(r.peek_rest());
        let value: Value = fastnbt::from_reader_with_opts(&mut cursor, DeOpts::network_nbt())
            .map_err(|err| WireError::Nbt {
                field,
                message: err.to_string(),
            })?;
        // `position` is bounded by the slice it read from, so the cast cannot lose anything.
        r.take(field, cursor.position() as usize)?;
        Ok(Self(value))
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        _: ProtocolVersion,
        field: &'static str,
    ) -> WireResult<()> {
        let bytes =
            fastnbt::to_bytes_with_opts(&self.0, SerOpts::network_nbt()).map_err(|err| {
                WireError::Nbt {
                    field,
                    message: err.to_string(),
                }
            })?;
        w.raw(&bytes);
        Ok(())
    }
}

/// A chat component, held as the JSON the rest of the world speaks.
///
/// On the wire it is NBT (from 1.20.3 on), in one of two shapes: a bare `TAG_String` for a literal
/// piece of text, and a compound for anything with formatting. A value that does not start with `{`
/// is written as the former and read back as itself; everything else round trips through JSON.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TextComponent(pub String);

impl TextComponent {
    /// Creates a component from literal text, which is written as a bare `TAG_String`.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl From<&str> for TextComponent {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

impl From<String> for TextComponent {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl Property for TextComponent {
    const NAME: &'static str = "text_component";

    fn decode(
        r: &mut Reader<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> WireResult<Self> {
        if r.peek_rest().first() != Some(&TAG_STRING) {
            let Nbt(value) = Nbt::decode(r, version, field)?;
            let json = serde_json::to_string(&value).map_err(|err| WireError::Nbt {
                field,
                message: err.to_string(),
            })?;
            return Ok(Self(json));
        }

        // A literal component is short-cut rather than walked, because it is the one the router
        // sends for every disconnect reason it writes itself.
        r.u8(field)?;
        let length = r.u16(field)? as usize;
        let bytes = r.take(field, length)?;
        String::from_utf8(bytes.to_vec())
            .map(Self)
            .map_err(|_| WireError::Utf8 { field })
    }

    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> WireResult<()> {
        if self.0.starts_with('{') {
            let value: Value = serde_json::from_str(&self.0).map_err(|err| WireError::Nbt {
                field,
                message: err.to_string(),
            })?;
            return Nbt(value).encode(w, version, field);
        }

        let bytes = self.0.as_bytes();
        if bytes.len() > TAG_STRING_LIMIT {
            return Err(WireError::LengthLimit {
                field,
                limit: TAG_STRING_LIMIT,
                actual: bytes.len(),
            });
        }
        w.u8(TAG_STRING);
        // Bounded by the check above, so the cast is lossless.
        w.u16(bytes.len() as u16);
        w.raw(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    /// Round trips a property through the writer and the reader it will meet.
    fn round_trip<T: Property + std::fmt::Debug>(value: &T) -> T {
        let mut buf = BytesMut::new();
        value
            .encode(
                &mut Writer::new(&mut buf),
                ProtocolVersion::UNKNOWN,
                "value",
            )
            .expect("encodes");
        let mut r = Reader::new(&buf);
        let decoded = T::decode(&mut r, ProtocolVersion::UNKNOWN, "value").expect("decodes");
        r.finish("Test").expect("consumes the whole payload");
        decoded
    }

    #[test]
    fn literal_text_is_a_bare_tag_string() {
        let component = TextComponent::text("Server full");
        let mut buf = BytesMut::new();
        component
            .encode(
                &mut Writer::new(&mut buf),
                ProtocolVersion::UNKNOWN,
                "reason",
            )
            .expect("encodes");
        // A tag byte, a `u16` length and the text: the shortest disconnect reason there is.
        assert_eq!(&buf[..3], &[TAG_STRING, 0x00, 0x0B]);
        assert_eq!(round_trip(&component), component);
    }

    #[test]
    fn a_formatted_component_round_trips_through_json() {
        let component = TextComponent(r#"{"text":"Server full"}"#.to_owned());
        let decoded = round_trip(&component);
        // The compound is not ordered, so the JSON is compared by value rather than by text.
        let expected: serde_json::Value = serde_json::from_str(&component.0).expect("is JSON");
        let actual: serde_json::Value = serde_json::from_str(&decoded.0).expect("is JSON");
        assert_eq!(actual, expected);
    }

    #[test]
    fn an_nbt_value_reads_only_its_own_bytes() {
        // The field that made this worth walking: a component in the middle of a packet, with
        // another field behind it.
        let mut buf = BytesMut::new();
        let mut w = Writer::new(&mut buf);
        TextComponent::text("label")
            .encode(&mut w, ProtocolVersion::UNKNOWN, "label")
            .expect("encodes");
        w.string("url", "https://justchunks.net").expect("encodes");

        let mut r = Reader::new(&buf);
        let label: TextComponent = r
            .property(ProtocolVersion::UNKNOWN, "label")
            .expect("decodes");
        assert_eq!(label, TextComponent::text("label"));
        assert_eq!(
            r.string("url", 255).expect("decodes"),
            "https://justchunks.net"
        );
        r.finish("Test").expect("consumes the whole payload");
    }

    #[test]
    fn a_payload_that_is_not_nbt_names_the_field_it_broke() {
        // 0x7F is not a tag, so the walk fails before it allocates anything.
        let err = Nbt::decode(
            &mut Reader::new(&[0x7F, 0x00]),
            ProtocolVersion::UNKNOWN,
            "dialog",
        )
        .expect_err("must reject");
        assert!(
            matches!(
                err,
                WireError::Nbt {
                    field: "dialog",
                    ..
                }
            ),
            "{err}"
        );
    }
}
