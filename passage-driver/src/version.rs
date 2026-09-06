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
//! # Snapshots are not versions you can compare
//!
//! Snapshot builds encode their protocol version with bit 30 set (`0x4000_0000 | n`), so *every*
//! snapshot compares above *every* release. A plain `>=` therefore says a 1.20.5 snapshot is newer
//! than the 26.2 protocol, and every version-gated field would be written for a client that cannot
//! read it. [`ProtocolVersion::is_release`] is how a handler refuses them; see
//! [`MIN_LOGIN_VERSION`](crate::demo::server::MIN_LOGIN_VERSION) for the shape.
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
    ///
    /// It is meaningful only for a version that [`is_release`](ProtocolVersion::is_release): a
    /// snapshot is numerically above every release and would cross every threshold.
    #[must_use]
    pub const fn at_least(self, other: Self) -> bool {
        self.0 >= other.0
    }

    /// Whether this is a snapshot build.
    ///
    /// Snapshots set bit 30 of the protocol number, which puts all of them above all releases. They
    /// are not ordered against releases in any useful way, so nothing gated on
    /// [`at_least`](ProtocolVersion::at_least) can be resolved for one.
    #[must_use]
    pub const fn is_snapshot(self) -> bool {
        self.0 & SNAPSHOT_BIT != 0
    }

    /// Whether this version can be compared against the version table at all.
    ///
    /// False for snapshots and for the negative numbers a client is free to send.
    /// [`UNKNOWN`](ProtocolVersion::UNKNOWN) counts as comparable: it is below every threshold,
    /// which is exactly what the pre-handshake state needs.
    #[must_use]
    pub const fn is_release(self) -> bool {
        self.0 >= 0 && !self.is_snapshot()
    }

    /// Where this version sits in a version table: itself when it can be ordered against the
    /// thresholds, and [`UNKNOWN`](ProtocolVersion::UNKNOWN) when it cannot.
    ///
    /// This is the *one* statement of "what do we do with a version we cannot place", and both
    /// directions go through it: [`ids`](crate::packet::ids) resolves outbound packet IDs against
    /// it, and [`Router`](crate::router::Router) picks its dispatch table with it. They used to
    /// answer differently -- a snapshot dispatched against the floor table and encoded against the
    /// newest IDs, so an ID we accepted was not one we would have sent, and a client on a negative
    /// version could be *dispatched* a status request and then not be *answered*, because no
    /// packet resolved for it.
    ///
    /// The floor is the honest answer for both: it holds exactly the packets that do not depend on
    /// a version at all, which is enough to answer a ping and to say why a login was refused.
    #[must_use]
    pub const fn placed(self) -> Self {
        if self.is_release() {
            self
        } else {
            Self::UNKNOWN
        }
    }
}

/// The bit a snapshot sets in its protocol number.
const SNAPSHOT_BIT: i32 = 0x4000_0000;

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
        // Which is precisely why neither is a version a codec may be resolved against.
        assert!(!ProtocolVersion::new(i32::MIN).is_release());
        assert!(!ProtocolVersion::new(i32::MAX).is_release());
    }

    #[test]
    fn a_snapshot_outranks_every_release_and_is_refused_for_it() {
        // 24w03a, a 1.20.5 snapshot: numerically above the 26.2 protocol, which is exactly the
        // trap. Anything gated on `at_least` would be written for a client that cannot read it.
        let snapshot = ProtocolVersion::new(0x4000_0000 | 132);
        assert!(snapshot.at_least(versions::V26_2));
        assert!(snapshot.is_snapshot());
        assert!(!snapshot.is_release());

        // Releases are not snapshots, and neither is the pre-handshake floor.
        for version in [versions::V1_20_5, versions::V1_21, versions::V26_2] {
            assert!(version.is_release(), "{version}");
        }
        assert!(ProtocolVersion::UNKNOWN.is_release());
    }

    #[test]
    fn a_version_that_cannot_be_placed_falls_back_to_the_floor() {
        // Everything a codec could be resolved against keeps its own place.
        for version in [
            ProtocolVersion::UNKNOWN,
            versions::V1_20_5,
            versions::V1_21,
            versions::V26_2,
        ] {
            assert_eq!(version.placed(), version, "{version}");
        }
        // Everything else lands on the floor, in *both* directions -- which is the point: the
        // table a connection dispatches against and the IDs it encodes with have to agree.
        for version in [
            ProtocolVersion::new(0x4000_0000 | 132),
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
        for version in [versions::V1_20_5, versions::V1_21, versions::V26_2] {
            assert!(!ProtocolVersion::UNKNOWN.at_least(version), "{version}");
        }
    }
}
