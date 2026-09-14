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

    /// The 26.2 protocol: Added the session ID field to `Login Success`.
    pub const V26_2: V = V::new(775);
}
