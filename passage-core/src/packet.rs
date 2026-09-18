use crate::direction::Direction;
use crate::phase::Phase;
use crate::version::ProtocolVersion;
use crate::wire::{Reader, WireResult, Writer};

/// A protocol packet.
pub trait Packet: Sized + Send + Sync + 'static {
    /// The name of the packet, used for tracing, metrics and error messages.
    const NAME: &'static str;

    /// The phase this packet belongs to.
    const PHASE: Phase;

    /// The direction this packet travels in.
    const DIRECTION: Direction;

    /// The ID of this packet in each version that changed it, **ordered newest to oldest**:
    ///
    /// ```ignore
    /// const IDS: &[(ProtocolVersion, i32)] =
    ///     &[(versions::V1_21_2, 0x03), (versions::V1_20_5, 0x02)];
    /// ```
    ///
    /// Data rather than a function, because the router does not only *ask* for IDs -- it reads the
    /// thresholds to work out where the protocol changes shape, and builds one dispatch table per
    /// change rather than one per version anybody remembered to list. See
    /// [`RouterBuilder::build`](crate::router::RouterBuilder::build).
    ///
    /// The order is load-bearing and checked at build time
    /// ([`RouterError::UnorderedIds`](crate::router::RouterError::UnorderedIds)): a table written
    /// oldest-first would silently resolve to the wrong ID.
    const IDS: &'static [(ProtocolVersion, i32)];

    /// Decodes the packet payload (without length prefix).
    ///
    /// The caller checks that the whole payload was consumed, so a decoder does not call
    /// [`Reader::finish`] itself.
    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> WireResult<Self>;

    /// Encodes the packet payload (without length prefix).
    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> WireResult<()>;

    /// The packet ID in `version`, or [`None`] if the packet does not exist there.
    ///
    /// Resolved from [`IDS`](Packet::IDS); there is no reason to override it.
    #[must_use]
    fn id(version: ProtocolVersion) -> Option<i32> {
        ids(version, Self::IDS)
    }
}

/// Resolves a packet ID from a table of `(first version, id)` pairs, **ordered newest to oldest**.
///
/// The first entry whose version is `<=` the connection's version wins. If none matches, the packet
/// does not exist in that version and the result is [`None`] -- which makes sending it an internal
/// error instead of a malformed frame, and keeps it out of the decode table entirely.
///
/// A version the table cannot order -- a snapshot, a negative number -- resolves against the floor
/// instead, exactly as [`Router`](crate::router::Router) picks its dispatch table. See
/// [`ProtocolVersion::placed`] for why the two have to agree.
#[must_use]
pub fn ids(version: ProtocolVersion, table: &[(ProtocolVersion, i32)]) -> Option<i32> {
    let version = version.placed();
    table
        .iter()
        .find(|(since, _)| version.at_least(*since))
        .map(|(_, id)| *id)
}

/// Finds the first out-of-order id in the packet ids definition. The packet ids should be ordered
/// by the protocol version.
#[must_use]
pub fn check_ids_unordered(
    ids: &'static [(ProtocolVersion, i32)],
) -> Option<(ProtocolVersion, ProtocolVersion)> {
    ids.windows(2)
        .find_map(|pair| (pair[0].0 <= pair[1].0).then_some((pair[0].0, pair[1].0)))
}
