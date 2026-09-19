use std::fmt;

/// The bit a snapshot sets in its protocol number.
const SNAPSHOT_BIT: i32 = 0x4000_0000;

/// A Minecraft (Java) protocol version, as sent in the handshake `intention` packet. The handshake
/// protocol version is compared against a set of breakpoints which define breaking changes in the
/// protocol that have to be handled by the wire codec.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ProtocolVersion(i32);

impl ProtocolVersion {
    /// The version of a connection that has not sent its handshake yet. It is below every real version,
    /// which is what makes it a safe starting point.
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

    /// Whether this version is at least `other` (inclusive lower bound). This is only meaningful
    /// between [`release`](ProtocolVersion::is_release) versions, as snapshots are numerically above
    /// every release version.
    #[must_use]
    pub const fn at_least(self, other: Self) -> bool {
        self.0 >= other.0
    }

    /// Whether this is a snapshot build.
    #[must_use]
    pub const fn is_snapshot(self) -> bool {
        self.0 & SNAPSHOT_BIT != 0
    }

    /// Whether this version is a release and can be compared against other versions. This includes
    /// the initial [`UNKNOWN`](ProtocolVersion::UNKNOWN) version.
    #[must_use]
    pub const fn is_release(self) -> bool {
        self.0 >= 0 && !self.is_snapshot()
    }

    /// Gets the version placement, moving all non-[`release`](ProtocolVersion::is_release) versions
    /// into [`UNKNOWN`](ProtocolVersion::UNKNOWN).
    #[must_use]
    pub const fn placed(self) -> Self {
        if self.is_release() {
            self
        } else {
            Self::UNKNOWN
        }
    }
}

impl From<i32> for ProtocolVersion {
    fn from(raw: i32) -> Self {
        Self(raw)
    }
}

impl AsRef<i32> for ProtocolVersion {
    fn as_ref(&self) -> &i32 {
        &self.0
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// TODO move into router implementation where these are actually used
/// The known protocol versions (breakpoints). It only contains release versions that introduced breaking
/// changes, mapping them against readable minecraft version names.
pub mod versions {
    use super::ProtocolVersion as V;

    /// Minecraft 1.20.5: introduced the configuration phase, cookies, and the transfer packet.
    /// This is the oldest version Passage (router) can serve.
    pub const V1_20_5: V = V::new(766);

    /// Minecraft 1.21.2: added the `Custom Report Details` and `Server Links` packets, and the
    /// particle status field to `Client Information`.
    pub const V1_21_2: V = V::new(768);

    /// Minecraft 1.21.6: added the dialog packets (`Clear Dialog`, `Show Dialog` and
    /// `Custom Click Action`).
    pub const V1_21_6: V = V::new(771);

    /// Minecraft 1.21.9: added the code of conduct packets.
    pub const V1_21_9: V = V::new(773);

    /// The 26.1 protocol: Added the session ID field to `Login Success`.
    pub const V26_1: V = V::new(775);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1.21: a release between the two breakpoints, which no list in this crate names.
    const V1_21: ProtocolVersion = ProtocolVersion::new(767);

    /// 24w03a, a 1.20.5 snapshot: numerically above every release, which is the trap.
    const SNAPSHOT: ProtocolVersion = ProtocolVersion::new(0x4000_0000 | 132);

    #[test]
    fn thresholds_are_inclusive_lower_bounds() {
        assert!(!V1_21.at_least(versions::V26_1));
        assert!(versions::V26_1.at_least(versions::V26_1));
        assert!(versions::V26_1.at_least(versions::V1_20_5));
        // 1.20.4: below the oldest version Passage can serve.
        assert!(!ProtocolVersion::new(765).at_least(versions::V1_20_5));
    }

    #[test]
    fn hostile_versions_do_not_cross_a_threshold_by_accident() {
        // A client is free to send a negative or absurd version. Neither may cross a threshold
        // unexpectedly, and neither may panic.
        assert!(!ProtocolVersion::new(i32::MIN).at_least(versions::V1_20_5));
        assert!(ProtocolVersion::new(i32::MAX).at_least(versions::V1_20_5));
        // Which is precisely why neither is a version a codec may be resolved against.
        assert!(!ProtocolVersion::new(i32::MIN).is_release());
        assert!(!ProtocolVersion::new(i32::MAX).is_release());
    }

    #[test]
    fn a_snapshot_outranks_every_release_and_is_refused_for_it() {
        // Anything gated on `at_least` would be written for a client that cannot read it.
        assert!(SNAPSHOT.at_least(versions::V26_1));
        assert!(SNAPSHOT.is_snapshot());
        assert!(!SNAPSHOT.is_release());

        // Releases are not snapshots, and neither is the pre-handshake floor.
        for version in [versions::V1_20_5, V1_21, versions::V26_1] {
            assert!(version.is_release(), "{version}");
        }
        assert!(ProtocolVersion::UNKNOWN.is_release());
        assert!(!ProtocolVersion::UNKNOWN.is_snapshot());
    }

    #[test]
    fn a_version_that_cannot_be_placed_falls_back_to_the_floor() {
        // Everything a codec could be resolved against keeps its own place.
        for version in [
            ProtocolVersion::UNKNOWN,
            versions::V1_20_5,
            V1_21,
            versions::V26_1,
        ] {
            assert_eq!(version.placed(), version, "{version}");
        }
        // Everything else lands on the floor, in *both* directions -- which is the point: the
        // table a connection dispatches against and the IDs it encodes with have to agree.
        for version in [
            SNAPSHOT,
            ProtocolVersion::new(-1),
            ProtocolVersion::new(i32::MIN),
            ProtocolVersion::new(i32::MAX),
        ] {
            assert_eq!(version.placed(), ProtocolVersion::UNKNOWN, "{version}");
        }
    }

    #[test]
    fn unknown_is_below_every_real_version() {
        // Load-bearing: it is what makes the pre-handshake state resolve to the version-independent
        // packets and nothing else.
        for version in [versions::V1_20_5, V1_21, versions::V26_1] {
            assert!(!ProtocolVersion::UNKNOWN.at_least(version), "{version}");
        }
    }

    #[test]
    fn a_raw_version_survives_the_round_trip() {
        for raw in [0, 765, 775, -1, i32::MIN, i32::MAX] {
            let version = ProtocolVersion::from(raw);
            assert_eq!(version.get(), raw);
            assert_eq!(*version.as_ref(), raw);
            assert_eq!(version.to_string(), raw.to_string());
        }
    }
}
