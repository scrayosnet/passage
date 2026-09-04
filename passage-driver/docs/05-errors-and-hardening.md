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
    Peer,        // the peer sent something illegal, or stopped playing along: logged at debug
    Transport,   // the connection broke: not actionable
    Internal,    // our bug or a dependency failing: warn and report
}

pub enum Completion {
    Closed,      // a handler ended it (status answered, transfer sent, disconnect sent)
    PeerClosed,  // the peer hung up
    Cancelled,   // a shutdown
    TimedOut,    // a deadline expired
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
Eof · VarIntTooLong · VarIntNotCanonical · NegativeLength · LengthLimit · TrailingBytes
Utf8 · InvalidValue · FrameTooLarge · UnknownPacket · UnexpectedPacket · EarlyPacket
UnsupportedVersion
```

### Handlers must be able to classify their own failures

The taxonomy above is only worth having if the layer above can use it, and for a while it could not:
the only channel a handler had was `InternalError::Handler(Box<dyn Error>)`, hard-wired to
`Class::Internal`. The demo showed exactly what that costs:

```rust
// a client that stops answering keep-alives -- reported as our bug
return Flow::fail(Error::Internal(InternalError::Handler(
    "client missed a keep-alive".into(),
)));
```

A timeout is ordinary client behaviour. Classified `Internal`, it is logged at `warn` and reported to
Sentry: the taxonomy's whole purpose, inverted, in the reference implementation of it. So the handler
supplies both the blame and the label:

```rust
pub enum Error {
    Protocol(ProtocolError),
    Transport(std::io::Error),
    Internal(InternalError),
    Handler {
        class: Class,
        label: &'static str,   // low-cardinality, never peer-controlled, like every other label
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    Closed,
}

// at the call site
Err(Error::peer("keep_alive_timeout", "the client missed a keep-alive"))
```

`Error` ended up *smaller* for it: `InternalError::Handler`'s other two users were a `JoinError`
(gone with the task model, see [04-runtime.md](04-runtime.md#why-not-joinset)) and a `format!` string
boxed into an error on the router's startup path -- which is now a `BuildError`.

### Wiring mistakes are not connection errors

```rust
pub enum BuildError {
    WrongDirection { packet, actual, expected },
    IdOutOfRange { packet, id, version },
    IdCollision { first, second, id, phase, version },
    TooManyPackets { count, limit },
}
```

A build error does not depend on any input, is the same on every run, and is discovered once:
`RouterBuilder::build` either produces a router that works for every supported version or it fails at
startup. Keeping these out of `Error` is why `Driver::new` cannot fail at all, and why there is no
`validate()` call to forget.

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
// in the router's erased decoder: once, for every packet
let packet = P::decode(&mut reader, version)?;
reader.finish(P::NAME)?;
```

If a packet decodes with bytes left over, our field list and the peer's disagree. Continuing means
every later packet is interpreted under a wrong assumption. This also catches our own mistakes when a
version gains a field we have not modelled yet -- which is exactly how the 26.2 change should have
surfaced.

Note *where* the check lives. It used to be emitted into every generated decoder; now that decoders
are written by hand it runs one level up, in dispatch, so no decoder can forget it or apply it
inconsistently. Moving a per-instance obligation into a single choke point is worth more than
generating it correctly N times.

### Rule 4 -- Every field states its own limit

```rust
server_address: r.string("server_address", 255)?,     // a hostname
body:           r.string("body", 32_768)?,            // a status JSON blob
properties:     r.array("properties", 16, version)?,  // vanilla sends one
```

The earlier design had a crate-wide `Limits::max_string_len` of `32_767 * 3` = 98 301 bytes, applied
to every string in every packet, because a generated codec has no way to say anything else. One
number for a hostname and a chat component is simultaneously far too loose and, eventually, too
tight.

So `Limits` shrank to the two things that genuinely are connection-wide:

```rust
pub struct Limits {
    pub max_frame_len: usize,      // 8 KiB -- enforced in *both* directions
    pub canonical_varints: bool,   // true
}
```

`max_string_len` and `max_array_len` are gone rather than deprecated: once every field named its own
bound, nothing read them. `max_frame_len` bounds all of them transitively anyway -- no field can be
longer than the frame carrying it -- so the per-field number is a tightening, never the only defence.

A status-only route can still afford a much smaller `max_frame_len` than a login route, which is why
limits remain configuration rather than constants.

### Rule 5 -- The outbound path checks its casts too

The reader honours rule 1 scrupulously; the writer used to do this:

```rust
writer.var_int(item.bytes.len() as i32);   // wraps above 2 GiB, and no frame-size check
```

Three of those existed (frame length, byte-slice length, array length). None was reachable, because
everything Passage sends it built itself -- but "unreachable given current callers" is not the
standard the module sets, and the failure mode is a length prefix that disagrees with its payload,
which is the hardest kind of protocol bug to diagnose from the other end. Lengths now go through one
checked path:

```rust
pub fn length(&mut self, value: usize) -> Result<()> {
    if value > self.limits.max_frame_len {
        return Err(InternalError::OversizedFrame { packet: self.packet, length: value, limit: .. }.into());
    }
    self.var_int(value as i32);   // bounded, so lossless
    Ok(())
}
```

It is an `InternalError` and it names the packet, because reaching it means we built something we
should not have. `Writer` carries the packet name for exactly this message.

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

Background tasks are polled on the connection's own task rather than spawned, so a panic in one takes
that connection down instead of being caught as a `JoinError`. That is a deliberate trade for a much
smaller task model; see [04-runtime.md](04-runtime.md#why-not-joinset). Whoever spawns the connection
observes the panic either way, and under the rules above a panic on the parse path is a bug rather
than something a peer can reach.

## Resource budgets

Two are now in the driver, because it owns the clock and the socket:

| Budget                        | Status | Why                                                                              |
|-------------------------------|--------|----------------------------------------------------------------------------------|
| `max_idle`                    | done   | A peer that connects and says nothing costs a task and a socket for free          |
| `max_lifetime`                | done   | The idle timer resets on every frame, so a peer could hold a connection open forever by pinging; this is also the backstop for a stalled exclusive task |
| Max packets per phase         | open   | Bounds "send `ClientInformation` a million times" without per-packet rate limits  |
| Max total bytes               | open   | Bounds the cheap version of the same attack                                       |
| Per-phase deadline            | open   | The handshake should take milliseconds; a global cap is coarse for that            |
| Concurrent connections per IP | keep   | Already present as `RateLimiter`; keep it, and count *rejections* as a metric      |

Both implemented budgets end the connection as `Completion::TimedOut` -- a completion, not an error,
because a client that went away is not a failure. Note that the caller cannot substitute
`tokio::time::timeout(driver.run())` for these: that drops the future mid-flight, so the shutdown
path never runs and background tasks are not cancelled cleanly.

## Testing this layer

| Technique                        | What it catches                                                                 |
|----------------------------------|---------------------------------------------------------------------------------|
| Unit tests per hostile input      | Regressions on the four known defects (implemented, `wire::tests`)               |
| Round-trip tests per packet per version | Codec asymmetry between encode and decode -- load-bearing now that codecs are hand-written (implemented, `demo::packets::tests::every_packet_roundtrips_in_every_version`) |
| Golden-byte tests                 | Silent wire-format changes that still round-trip                                  |
| `cargo fuzz` over `FrameCodec` + `Router::dispatch` | Everything above, plus the combinations nobody thought of; the target is ~20 lines and needs no network |
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
