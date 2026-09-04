# 1. What the current implementation gets wrong

This is the input to every other proposal. Each item is something the new structure has to make
impossible, not merely fixed once.

## 1.1 Packet identity is a constant

`passage-packets/src/lib.rs:321`

```rust
pub trait Packet {
    /// Returns the defined ID of this network packet.
    const ID: VarInt;
}
```

A `const` cannot depend on the connection's protocol version, so the crate can only ever speak one
version. The consequence is visible in the most recent commit (`1571caf`, "add 26.2 protocol
quickfix"), which added a field to `LoginSuccess` unconditionally:

`passage-packets/src/login.rs:117`

```rust
impl WritePacket for LoginSuccessPacket {
    fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
        dst.write_uuid(&self.user_id)?;
        dst.write_string(&self.user_name)?;
        dst.write_varint(0)?;                 // no properties

        // TODO elevate to packet field in the future (with backward compatability)
        dst.write_uuid(&Uuid::new_v4())?;     // <- 26.2 only; corrupts the frame for older clients
        Ok(())
    }
}
```

Two problems, both structural rather than accidental:

1. Every client below the 26.2 protocol now receives 16 bytes it does not expect.
2. The value is a fresh random UUID rather than anything meaningful, because there is nowhere to put
   a session ID -- the field does not exist on the type.

The commit message says as much ("This is not backward compatible with previous versions"). The
version-keyed ID table and gated fields in [02-versioning.md](02-versioning.md) exist to make this
particular kind of quick-fix unnecessary.

## 1.2 The wire truth is duplicated

Decoding needs the reverse mapping of `Packet::ID`, and today that lives in a macro at each call
site:

`passage-protocol/src/connection.rs:272`

```rust
let handshake = match_packet! { packet,
    packet = hand_in::HandshakePacket => packet,
    (unexpected, _) => { /* close */ }
}?;
```

The early `SCRATCH.md` sketch (since folded into these documents) already flagged the consequence:

> ```rust
> // TODO: The match should be defined at once place, not two (other is the packet)
> ```

With per-version IDs the duplication gets worse: every `match_packet!` arm would need its own
version condition. Deriving the decode table from the packet declaration removes the second place
entirely.

## 1.3 One 470-line function is the protocol

`Connection::listen` (`passage-protocol/src/connection.rs:263-730`) is the entire state machine:
handshake, routing, status, login, cookies, encryption, authentication, keep-alives, target
selection and transfer, in one linear function with a `select!` loop embedded in the middle.

It reads well -- that is genuinely a strength, and one the replacement must not throw away -- but:

* there is no way to add or replace a step without editing the function;
* there is no client implementation, and no way to write one from these parts;
* the sequential shape hides the concurrent part (target selection) in the middle of it;
* the loop's `if self.client_locale.is_some()` guard on the target-selection arm means the target is
  only ever collected after a `ClientInformation` packet arrives. A client that never sends one
  keeps the connection sending keep-alives until the global connection timeout, holding the backend
  slot the selection reserved.

## 1.4 The wire readers are not hardened

Verified by running the current code paths against hostile input:

| Input                              | Current behaviour                                                    | Location                          |
|------------------------------------|----------------------------------------------------------------------|-----------------------------------|
| `FF FF FF FF 0F` as a string length | `read_varint` returns `-1`; `length as usize` is `18446744073709551615`; `vec![0; length]` panics with *capacity overflow* | `reader.rs:82-87`, `reader.rs:117-122` |
| `FF FF FF FF 07` as a string length | `vec![0; 2147483647]` -- a 2 GiB allocation from a 10 byte packet     | `reader.rs:82-87`                 |
| 6-byte VarInt (`80 80 80 80 80 01`) | loop ends after five bytes, continuation byte left in the stream; every following field is misaligned | `reader.rs:56-67`                 |
| 10-byte VarLong (e.g. `-1`)         | `for i in 0..9` reads only nine bytes: value truncated to `i64::MAX` *and* one byte left in the stream | `reader.rs:69-80`                 |

The panic is reachable pre-authentication with a handshake packet, and it aborts the connection task
rather than the process -- but it is peer-triggerable, and with the Sentry panic integration enabled
it also becomes peer-triggerable error reporting.

The root cause is a pattern, not four bugs: **a length is used before it is validated**, and
validation is written per call site rather than being the only way to read a length. See
[05-errors-and-hardening.md](05-errors-and-hardening.md).

## 1.5 Errors double as control flow

`Error::ConnectionClosed` means both "we finished" and "we failed":

```rust
// passage-protocol/src/connection.rs:337 -- status exchange complete
return Ok(());

// passage-protocol/src/connection.rs:185 -- disconnect packet sent on purpose
self.send_packet(login_out::DisconnectPacket { reason }).await?;
Err(Error::ConnectionClosed)

// passage-protocol/src/listener.rs:165 -- so the caller has to treat them identically
match connection.listen().await {
    Ok(()) | Err(Error::ConnectionClosed) => debug!("connection completed"),
    Err(err) => warn!(cause = err.to_string(), "failed to handle connection"),
}
```

Consequences:

* Every caller must know that one error variant is not an error.
* There is no distinction between "a scanner sent garbage" (expected, `debug`, a counter) and "we
  produced a malformed packet" (a bug, `warn`, report it). Both land in the same `warn!`.
* Adding an error variant silently puts it in the wrong bucket.

Related: the connection path still contains `expect` calls that assume invariants
(`connection.rs:259` on cipher construction, `connection.rs:460` and `:685` on system time).

## 1.6 Direction is a Cargo feature

`passage-packets` gates `ReadPacket`/`WritePacket` behind the `client` and `server` features:

```rust
#[cfg(feature = "server")]
impl WritePacket for DisconnectPacket { ... }
#[cfg(feature = "client")]
impl ReadPacket for DisconnectPacket { ... }
```

For a library that wants to serve a server, a client and tests, this is the wrong axis: the
direction of a packet is a property of the packet, and anything that talks to both sides needs both
halves anyway. It also means the test configuration is the only one where a packet can round-trip,
so a `server`-only build cannot verify its own encoders.

## 1.7 What is worth keeping

Not everything needs replacing. These are good and the proposals preserve them:

* **The frame codec shape.** `PacketFrame` with a `Bytes` payload and a `tokio_util` codec, so
  unknown packets cost nothing and reads are cancellation-safe.
* **In-place encryption with a `decrypted_until` cursor.** Correct and allocation-free.
* **Adapter traits.** The five adapter traits are a good seam; nothing here changes them.
* **The linear readability of `listen`.** The new structure keeps it by pausing input while an
  asynchronous handler runs, so the login flow still reads top to bottom
  ([04-runtime.md](04-runtime.md)).
* **Metrics and tracing coverage**, which should grow rather than shrink
  ([06-layering-and-telemetry.md](06-layering-and-telemetry.md)).
