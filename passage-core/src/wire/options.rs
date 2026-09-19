/// The wire policy options.
///
/// It configures and limits the reader and writer to reject obviously invalid encodings.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Maximum length of a single frame, excluding the length prefix. Enforced in both directions.
    ///
    /// Sized by the biggest thing a server legitimately *sends*, which is a status response
    /// carrying a favicon: base64 of a 64x64 PNG, plus a MOTD and a sample. The vanilla client caps
    /// that JSON at 32 767 characters, so a limit below it refuses ordinary content. It also bounds
    /// every field transitively: no field can be longer than the frame that carries it.
    pub max_frame_len: usize,

    /// Whether non-canonical (overlong) `VarInt`/`VarLong` encodings are rejected.
    ///
    /// Vanilla clients always encode minimally. Rejecting overlong forms removes a
    /// parser-differential surface, at the cost of being stricter than the reference
    /// implementation -- hence the switch.
    pub strict_varints: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_frame_len: 32 * 1024,
            strict_varints: true,
        }
    }
}

impl Options {
    /// Creates new permissive `Options`, allowing non-canonical (overlong) `VarInt`/`VarLong` encodings
    /// and not limiting the frame length. In general, the default options should be used.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            max_frame_len: usize::MAX,
            strict_varints: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_frame_limit_holds_a_status_response_with_a_favicon() {
        // The regression an 8 KiB default caused: a 64x64 favicon is base64 of a PNG that runs to
        // several kilobytes, and refusing it would have been our own error for content the client
        // asks for by default. The vanilla client caps that JSON at 32 767 characters.
        assert!(Options::default().max_frame_len >= 32 * 1024);
        assert!(Options::default().strict_varints);
    }

    #[test]
    fn permissive_options_are_permissive_in_both_respects() {
        let options = Options::permissive();
        assert_eq!(options.max_frame_len, usize::MAX);
        assert!(!options.strict_varints);
    }
}
