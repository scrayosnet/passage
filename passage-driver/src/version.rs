//! Protocol versions and the feature table derived from them.
//!
//! Passage supports a range of protocol versions with a *single* set of packet types. Two kinds of
//! change have to be modelled for that to work:
//!
//! 1. **Packet IDs move.** Handled by [`Packet::id`](crate::packet::Packet::id), which maps a
//!    version to the ID valid for that version (or [`None`] if the packet does not exist there).
//! 2. **Fields come and go.** Handled by *named* [`Feature`] gates. Codecs never compare raw
//!    version numbers -- they ask `version.has(Feature::X)`. Every raw number in the protocol lives
//!    in exactly one place: [`Feature::since`] and the [`versions`] table.
//!
//! Keeping the numbers out of the codecs is what makes this maintainable: when a field is
//! backported, moved or a version is added, only this module changes.

use std::fmt;

/// A Minecraft (Java) protocol version, as sent in the handshake `intention` packet.
///
/// The wire type is a `VarInt`, so negative values are representable and *must* be tolerated:
/// clients are free to send garbage. Comparisons are therefore always "at least", never equality.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ProtocolVersion(i32);

impl ProtocolVersion {
    /// The version of a connection that has not sent its handshake yet.
    ///
    /// Packets available at `UNKNOWN` are those whose encoding never changed and that have to be
    /// readable before the version is negotiated -- in practice only the handshake itself.
    pub const UNKNOWN: Self = Self(0);

    /// Wraps a raw protocol version number.
    #[must_use]
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    /// The raw protocol version number.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// Whether this version has the given [`Feature`].
    ///
    /// This is the *only* way codecs are allowed to branch on the version.
    #[must_use]
    pub const fn has(self, feature: Feature) -> bool {
        self.0 >= feature.since().0
    }

    /// Whether this version is at least `other`.
    #[must_use]
    pub const fn at_least(self, other: Self) -> bool {
        self.0 >= other.0
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The known protocol versions.
///
/// This table is the single place where release names are mapped to wire numbers. It is
/// deliberately sparse: only versions that a [`Feature`] or a packet ID actually keys off need an
/// entry. Add entries from <https://minecraft.wiki/w/Java_Edition_protocol/Packets>.
pub mod versions {
    use super::ProtocolVersion as V;

    /// Minecraft 1.20.5 -- introduced the configuration phase, cookies and the transfer packet.
    /// This is the oldest version Passage can serve at all.
    pub const V1_20_5: V = V::new(766);

    /// Minecraft 1.21.
    pub const V1_21: V = V::new(767);

    /// The 26.2 protocol -- added the session ID field to `Login Success`.
    pub const V26_2: V = V::new(775);
}

/// A named, version-gated protocol capability.
///
/// Each variant documents *what* changed and [`Feature::since`] records *when*. Codecs and
/// handlers refer to the variant, never to the number.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// Cookies, the `transfer` intent and the `Transfer` packet exist.
    Cookies,

    /// `Login Success` carries a trailing session ID (UUID) after the profile properties.
    LoginSuccessSessionId,
}

impl Feature {
    /// The first protocol version that has this feature.
    #[must_use]
    pub const fn since(self) -> ProtocolVersion {
        match self {
            Feature::Cookies => versions::V1_20_5,
            Feature::LoginSuccessSessionId => versions::V26_2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_gated_by_version() {
        assert!(!versions::V1_21.has(Feature::LoginSuccessSessionId));
        assert!(versions::V26_2.has(Feature::LoginSuccessSessionId));
        assert!(versions::V1_20_5.has(Feature::Cookies));
        assert!(!ProtocolVersion::new(765).has(Feature::Cookies));
    }

    #[test]
    fn hostile_versions_do_not_enable_features() {
        // A client is free to send a negative or absurd version. Neither may enable a feature by
        // accident, and neither may panic.
        assert!(!ProtocolVersion::new(i32::MIN).has(Feature::Cookies));
        assert!(ProtocolVersion::new(i32::MAX).has(Feature::Cookies));
    }
}
