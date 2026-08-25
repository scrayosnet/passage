# 5. Errors and hardening

Passage is an unauthenticated internet-facing parser. Everything before the encryption handshake runs
on bytes from anyone who can open a TCP connection. The goal is not "no bugs" but "the shape of the
code makes the dangerous thing hard to write".

## E. Error taxonomy

### E1 -- One flat enum (today)

```rust
pub enum Error {
    ConnectionClosed,     // ...which also means "success"
    NoRouteFound,
    Adapter(..), Packet(..), Crypto(..), Cookie(..),
}
```

| Pros                 | Cons                                                                                     |
|----------------------|------------------------------------------------------------------------------------------|
| Minimal              | The caller cannot tell a scanner from a bug, so everything is logged at the same level     |
|                      | One variant is not an error, and every call site must know that                            |
|                      | A new variant lands in whatever bucket the existing `match` happens to have                |

### E2 -- Blame-classified, with completion separated *(recommended)*

Two independent axes: *who caused it* and *did the connection finish*.

```rust
pub enum Class {
    Peer,        // the peer sent something illegal: expected, counted, logged at debug
    Transport,   // the connection broke: not actionable
    Internal,    // our bug or a dependency failing: warn and report
}

pub enum Completion {
    Closed,      // a handler ended it (status answered, transfer sent, disconnect sent)
    PeerClosed,  // the peer hung up
    Cancelled,   // shutdown or timeout
}
```

`Driver::run` returns `Result<Completion>`, so finishing is never an error:

```rust
match result {
    Ok(completion) => debug!(?completion, "connection finished"),
    Err(err) => match err.class() {
        Class::Peer | Class::Transport => debug!(cause = %err, kind = err.label(), "connection dropped"),
        Class::Internal               => warn!(cause = %err, kind = err.label(), "connection failed"),
    },
}
```

| Pros                                                                     | Cons                                                     |
|--------------------------------------------------------------------------|----------------------------------------------------------|
| A new error variant inherits the right log level from its class            | Two types where there was one                             |
| `label()` gives low-cardinality metric labels with no peer-controlled data | The classification has to be assigned honestly per variant |
| Sentry only sees `Internal`, so peer garbage cannot generate alert noise    |                                                          |
| Peer errors become a *signal* (a counter per label) instead of log spam     |                                                          |

`ProtocolError` variants are fine-grained on purpose -- `negative_length` and `frame_too_large` are
different stories about who is talking to you:

```rust
Eof · VarIntTooLong · VarIntNotCanonical · NegativeLength · LengthLimit
TrailingBytes · Utf8 · FrameTooLarge · UnknownPacket · UnexpectedPacket · UnsupportedVersion
```

## Hardening the wire layer

The four defects in [01-problem-analysis.md](01-problem-analysis.md#14-the-wire-readers-are-not-hardened)
share one cause: a length is used before it is validated, and validation is per call site. The fix is
to make the unvalidated path unavailable.

### Rule 1 -- Never allocate for bytes you have not received

```rust
pub fn length(&mut self, field: &'static str, limit: usize) -> Result<usize> {
    let raw = self.var_int()?;
    if raw < 0 {
        return Err(ProtocolError::NegativeLength { field, value: raw }.into());
    }
    let length = raw as usize;              // non-negative: lossless
    if length > limit {
        return Err(ProtocolError::LengthLimit { field, limit, actual: length }.into());
    }
    if length > self.remaining() {
        return Err(ProtocolError::Eof { needed: length, remaining: self.remaining() }.into());
    }
    Ok(length)
}
```

Every length-prefixed read goes through this, so the amplification factor of a hostile packet is
bounded by the frame size (8 KiB by default) rather than by `i32::MAX`. `Vec::with_capacity` for
arrays inherits the same bound: an element is at least one byte, so a claimed count above the
remaining bytes is rejected before anything is reserved.

### Rule 2 -- Reject what the reference implementation would never send

```rust
// five bytes maximum, and the fifth carries only four significant bits
if index == 4 && bits > 0b1111 {
    return Err(ProtocolError::VarIntTooLong { kind: "VarInt" }.into());
}
// overlong encodings give the same value several representations
if self.limits.canonical_varints && index > 0 && bits == 0 {
    return Err(ProtocolError::VarIntNotCanonical { kind: "VarInt" }.into());
}
```

Strictness is a `Limits` flag rather than a hard-coded choice, because "stricter than vanilla" is a
policy decision and a modded client is a plausible reason to relax it.

`VarLong` bounds are ten bytes, not nine: 64 bits do not divide into groups of seven, and stopping at
nine both truncates the value *and* leaves a byte in the stream -- the second half is the dangerous
one, because it silently shifts every following field.

### Rule 3 -- Trailing bytes are an error

```rust
reader.finish(Self::NAME)?;   // generated at the end of every decode
```

If a packet decodes with bytes left over, our field list and the peer's disagree. Continuing means
every later packet is interpreted under a wrong assumption. This also catches our own mistakes when a
version gains a field we have not modelled yet -- which is exactly how the 26.2 change should have
surfaced.

### Rule 4 -- Limits are configuration, not constants

```rust
pub struct Limits {
    pub max_frame_len: usize,      // 8 KiB
    pub max_string_len: usize,     // protocol maximum; individual fields should be far smaller
    pub max_array_len: usize,      // 1024
    pub canonical_varints: bool,   // true
}
```

A status-only route can afford much tighter limits than a login route. Per-field limits are still
worth adding (`server_address` needs 255 bytes, not 98 301).

## No-panic rules

The parse path must not panic, because a panic is a peer-triggerable abort of the connection task
and, with Sentry's panic integration, peer-triggerable error reporting.

* `#![deny(unsafe_code)]` at the crate root.
* No `unwrap`/`expect` on anything derived from peer input. Enforce with
  `#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]`.
* No slice indexing on peer-derived offsets; the reader's `take()` is the only place that indexes,
  and it checks first. `clippy::indexing_slicing` is worth enabling for the wire module.
* No unchecked arithmetic on lengths: `saturating_sub` for cursor maths, and
  `clippy::arithmetic_side_effects` for the codec.
* Shifts are bounded by construction in the varint readers (five and ten iterations).
* `Duration`/`SystemTime` arithmetic is our own data, but `expect("time error")` is still a panic; it
  belongs in the `Internal` class, not in an `expect`.

Panics that do slip through are contained: `JoinSet` reports a panicking detached task as
`JoinError`, which the driver turns into `Class::Internal` rather than letting it propagate.

## Resource budgets

Missing from the reference implementation and worth adding early:

| Budget                   | Why                                                                                     |
|--------------------------|-----------------------------------------------------------------------------------------|
| Per-phase deadline       | A connection idling in the login phase costs a task, a socket and (after selection) a backend slot |
| Max packets per phase    | Bounds "send `ClientInformation` a million times" without needing per-packet rate limits |
| Max total bytes          | Bounds the cheap version of the same attack                                              |
| Concurrent connections per IP | Already present as `RateLimiter`; keep it, and count *rejections* as a metric        |

The existing global `connection_timeout` covers the worst case but is far too coarse: the handshake
should take milliseconds.

## Testing this layer

| Technique                        | What it catches                                                                 |
|----------------------------------|---------------------------------------------------------------------------------|
| Unit tests per hostile input      | Regressions on the four known defects (implemented, `wire::tests`)               |
| Round-trip property tests per version | Codec asymmetry between encode and decode                                    |
| Golden-byte tests                 | Silent wire-format changes that still round-trip                                  |
| `cargo fuzz` over `FrameCodec` + `Bound::dispatch` | Everything above, plus the combinations nobody thought of; the target is ~20 lines and needs no network |
| Connection-level tests over `tokio::io::duplex` | Ordering, backpressure, completion classification (implemented, `tests/flow.rs`) |

A fuzz target is the highest-value missing piece: the decode path is a pure function from
`(version, phase, bytes)` to `Result`, which is the ideal fuzzing shape.

## Cookies and crypto: what to keep

The current cookie code is sound and should carry over as-is:

* HMAC-SHA256 with `mac.verify_slice`, which is a constant-time comparison.
* The signature covers the payload, and verification happens before deserialisation.

Two things to watch:

* The *session* cookie is deliberately unsigned and is parsed from peer input before any
  authentication. Keep its schema small and its size limited; `serde_json`'s default recursion limit
  is the only thing bounding nesting today.
* `verify_token` comparison should be constant-time as well, even though a timing oracle on a random
  32-byte token is not practically exploitable.
