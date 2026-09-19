use crate::ProtocolVersion;
use crate::wire::{Reader, WireResult, Writer};

/// A packet property field.
pub trait Property: Sized + Send + Sync + 'static {
    /// The name of the property, used for tracing, metrics, and error messages.
    const NAME: &'static str;

    /// Decodes the property.
    fn decode(
        r: &mut Reader<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> WireResult<Self>;

    /// Encodes the property
    fn encode(
        &self,
        w: &mut Writer<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> WireResult<()>;
}
