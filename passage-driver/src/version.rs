//! Protocol versions, and the one table that maps release names to wire numbers.
//!
//! Passage supports a range of protocol versions with a *single* set of packet types. Two kinds of
//! change have to be modelled for that to work, and both key off a version:
//!
//! 1. **Packet IDs move.** Handled by [`Packet::id`](crate::packet::Packet::id) via
//!    [`ids`](crate::packet::ids), which maps a version to the ID valid for that version (or
//!    [`None`] if the packet does not exist there).
//! 2. **Fields come and go.** Handled by a comparison in the codec:
//!    `version.at_least(versions::V26_2)`.
//!
//! The rule that makes this maintainable is not indirection, it is that **a codec never contains a
//! raw protocol number**. Every number lives in the [`versions`] table; codecs refer to its
//! constants. `grep -rn "at_least" src/` then lists every version-dependent field, and
//! `grep -rn "ids(" src/` every version-dependent ID -- together, a complete inventory.
//!
//! # Why there is no `Feature` enum
//!
//! An earlier revision routed every comparison through a named gate: a `Feature` variant, a
//! `Feature::since` arm mapping it to a version, and `version.has(Feature::X)` at the use site.
//! It was removed, for four reasons that only became clear once it had two variants:
//!
//! * **It grows one variant per changed field, forever.** Each is used once, and is defined in
//!   another module from the codec that uses it. A name used once, far from its use, is worse than
//!   the constant it stands for.
//! * **It hides the thing you actually want to know.** `version.has(Feature::LoginSuccessSessionId)`
//!   says *what*; `version.at_least(versions::V26_2)` says *when*. The protocol is documented,
//!   discussed and diffed by version, so when you implement a packet from the wiki the version is
//!   what you have in hand -- not a feature name someone invented for it.
//! * **Its stated benefit survives without it.** The point of the indirection was that a renumbered
//!   snapshot changes one line rather than every codec. A named constant does that too, and costs
//!   one line instead of an enum variant plus a match arm -- see [`versions`].
//! * **It had already rotted.** `Feature::Cookies` existed, mapped to 1.20.5, and was referenced by
//!   nothing but its own test, while
//!   [`MIN_LOGIN_VERSION`](crate::demo::server::MIN_LOGIN_VERSION) named the *same* threshold as a
//!   plain constant and got all the real use.

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
    /// It is below every real version, which is what makes it a safe starting point: no
    /// version-gated field and no version-specific packet ID resolves at `UNKNOWN`. What remains
    /// available is exactly the packets whose ID table starts here -- in practice the handshake and
    /// the status exchange.
    pub const UNKNOWN: Self = Self(0);

    /// Wraps a raw protocol version number.
    ///
    /// Reserved for decoding the handshake and for the [`versions`] table. A codec that calls this
    /// with a literal has put a protocol number somewhere it cannot be found again.
    #[must_use]
    pub const fn new(raw: i32) -> Self {
        Self(raw)
    }

    /// The raw protocol version number.
    #[must_use]
    pub const fn get(self) -> i32 {
        self.0
    }

    /// Whether this version is at least `other`.
    ///
    /// This is the *only* way to branch on a version. It is an inclusive lower bound, which matches
    /// how the protocol changes: a field appears in some version and stays.
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
/// This table is the single place where release names are mapped to wire numbers, and the only
/// place [`ProtocolVersion::new`] should appear with a literal. It is deliberately sparse: a
/// version needs an entry only once a packet ID or a field actually keys off it. Add entries from
/// <https://minecraft.wiki/w/Java_Edition_protocol/Packets>.
///
/// # Naming a threshold
///
/// Comparing against a version directly is the default, because the version *is* the explanation.
/// When a single change spans many packets, or the version it landed in is not obvious from the
/// call site, give the threshold a name -- as a constant, next to the code that uses it:
///
/// ```
/// use passage_driver::version::{ProtocolVersion, versions};
///
/// /// The oldest version that can log in: 1.20.5 introduced the configuration phase, cookies and
/// /// the transfer packet, all of which Passage depends on.
/// pub const MIN_LOGIN_VERSION: ProtocolVersion = versions::V1_20_5;
/// ```
///
/// That is one line, it lives where it is read, and a renumbered snapshot still changes only it.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_are_inclusive_lower_bounds() {
        assert!(!versions::V1_21.at_least(versions::V26_2));
        assert!(versions::V26_2.at_least(versions::V26_2));
        assert!(versions::V26_2.at_least(versions::V1_20_5));
        // 1.20.4: below the oldest version Passage can serve.
        assert!(!ProtocolVersion::new(765).at_least(versions::V1_20_5));
    }

    #[test]
    fn hostile_versions_do_not_cross_a_threshold_by_accident() {
        // A client is free to send a negative or absurd version. Neither may cross a threshold
        // unexpectedly, and neither may panic.
        assert!(!ProtocolVersion::new(i32::MIN).at_least(versions::V1_20_5));
        assert!(ProtocolVersion::new(i32::MAX).at_least(versions::V1_20_5));
    }

    #[test]
    fn unknown_is_below_every_real_version() {
        // Load-bearing: it is what makes the pre-handshake state resolve to the version-independent
        // packets and nothing else.
        for version in [versions::V1_20_5, versions::V1_21, versions::V26_2] {
            assert!(!ProtocolVersion::UNKNOWN.at_least(version), "{version}");
        }
    }
}
