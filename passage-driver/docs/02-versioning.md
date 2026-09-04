# 2. Protocol versions

Two independent problems hide behind "be backward compatible":

* **A. Identity.** The same packet has different IDs in different versions, and some packets do not
  exist in some versions at all.
* **B. Shape.** The same packet has different fields in different versions.

They need different mechanisms. Mixing them is how you end up with `LoginSuccessPacketV775`.

## A. Packet identity

### A1 -- One type per version

```rust
pub struct LoginSuccessPacket   { /* ... */ }   // <= 774
pub struct LoginSuccessPacket2  { /* ... */ }   // >= 775
```

| Pros                                             | Cons                                                                                          |
|--------------------------------------------------|-----------------------------------------------------------------------------------------------|
| Trivial to implement; each codec is exact         | Type explosion; every handler must match on all of them; a new version touches every call site |
| Old versions cannot be broken by a new one        | Shared logic has to be written against a trait or an enum, i.e. the problem moves              |

The README rules this out explicitly ("Do not create a new packet type for every protocol version").

### A2 -- `fn id(version) -> Option<VarInt>` plus a hand-written dispatcher

The state the early `SCRATCH.md` sketch reached:

```rust
pub trait Packet {
    fn id(version: ProtocolVersion) -> Option<VarInt>;
}

// ...and, separately:
match (version, frame.id) {
    (_, 0x04)   => PacketA::decode(&frame.data, version)?.accept(visitor),
    (775, 0x05) => PacketB::decode(&frame.data, version)?.accept(visitor),
}
```

| Pros                                        | Cons                                                                                     |
|---------------------------------------------|------------------------------------------------------------------------------------------|
| One type per packet, versions handled inside | The mapping exists twice, which the sketch itself marked as a TODO        |
| Explicit and greppable                      | The `match` grows as `packets x versions`; a missing arm is a runtime miss, not an error   |

### A3 -- Declarative ID table, both directions derived *(recommended)*

The packet declares its ID per version once. Encoding asks the packet; decoding asks every packet at
startup and builds a table per supported version.

```rust
impl Packet for LoginSuccess {
    const NAME: &'static str = "LoginSuccess";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Clientbound;

    // newest first; "from this version on, this id"
    fn id(version: ProtocolVersion) -> Option<i32> {
        ids(version, &[(versions::V1_20_5, 0x02)])
    }
    // ...
}
```

```rust
// built once at startup, for every supported version
let router = Router::builder(Direction::Serverbound)
    .on::<LoginSuccess, _>(..)
    .build(SUPPORTED_VERSIONS.iter().copied())?;   // (phase, id) -> handler, per version
```

A packet that moved is one more entry, still in one place:

```rust
fn id(version: ProtocolVersion) -> Option<i32> {
    ids(version, &[
        (versions::V26_2, 0x03),     // moved
        (versions::V1_20_5, 0x02),
    ])
}
```

| Pros                                                                                | Cons                                                              |
|-------------------------------------------------------------------------------------|-------------------------------------------------------------------|
| One source of truth; decode table cannot drift from the encoder                      | Ordering of the `ids` list is a convention nothing checks          |
| ID collisions are a *build* failure: `RouterBuilder::build` resolves every version    | The supported version list has to be maintained                    |
| "Does not exist in this version" is representable (`None`) and enforced when sending  |                                                                   |
| Dispatch is a table index, not a linear `match`; tables are shared, not per connection |                                                                  |

### A4 -- Generate from an external protocol database

Generate the tables in `build.rs` from `minecraft-data` or a scraped wiki dump.

| Pros                                                     | Cons                                                                             |
|----------------------------------------------------------|----------------------------------------------------------------------------------|
| Full coverage of every version for free                   | A build-time network/vendored dependency, and a supply-chain surface              |
| No manual work when a version lands                       | Generated code is hard to review; upstream errors become our bugs silently        |
|                                                          | Passage needs ~15 packets: the coverage is worth little, the opacity costs a lot  |

**Recommendation: A3.** Keep A4 in mind as a *checker* rather than a generator -- a test that
compares our tables against a vendored dump is cheap and catches typos without letting generated
code into the build.

### A note on how the codecs got written: the `packet!` macro, tried and removed

An earlier iteration of this crate generated the whole packet -- struct, `id`, `decode`, `encode` --
from a `packet!` macro. It worked, and it was removed. Three things it could not express turned out
to matter more than the ten lines per packet it saved:

* **Per-field limits.** A generated decoder can only reach one crate-wide string limit, so
  `server_address` and a chat component were both allowed ~98 KB. A hostname wants 255 bytes, and it
  is the field that should say so. (`Limits::max_string_len` and `max_array_len` are gone entirely
  now: nothing read them once every field named its own bound.)
* **Domain types.** Because the macro picked an encoding from a field's *type*, a `VarInt` field had
  to be a `VarInt` newtype in the struct, and validating it landed in the handler:
  `match packet.intent.0 { 1 => .., 2 => .., _ => Err(..) }`. Written out, the decoder produces an
  `Intent` and no handler can see an invalid one. The `VarInt`/`VarLong` newtypes are gone with it --
  the *call* picks the encoding (`r.var_int()`), which is where the choice belongs.
* **Version-dependent shape.** Reordered, split or retyped fields are an `if` in a hand-written
  codec. Under the macro they needed a second packet type -- the A1 outcome this document rejects.

What was worth keeping from the macro was moved rather than dropped: the trailing-bytes check it
emitted into every decoder now runs once, in the router's erased decoder, so it cannot be forgotten
*or* be inconsistent. And the ID table stayed declarative through a plain function (`ids`) instead
of macro syntax.

The cost is ~10 lines per packet and the fact that a hand-written encoder and decoder can now
disagree with each other. The second is the real one, and it is answered by a test rather than by a
macro: `every_packet_roundtrips_in_every_version` is one table-driven test over every packet × every
supported version. If the packet count ever reaches the hundreds, the escape hatch is A4 -- codegen
emits exactly this shape, and fits it far better than `macro_rules!` did.

## B. Version-dependent fields

### B1 -- Separate structs per shape

Covered above: rejected for the same reason as A1.

### B2 -- `Option<T>` fields gated by a named feature *(recommended)*

The README asks for optional fields. The important addition is that codecs must not compare version
numbers themselves:

```rust
// version.rs -- the only place a protocol number appears
pub enum Feature {
    Cookies,
    LoginSuccessSessionId,
}

impl Feature {
    pub const fn since(self) -> ProtocolVersion {
        match self {
            Feature::Cookies              => versions::V1_20_5,
            Feature::LoginSuccessSessionId => versions::V26_2,
        }
    }
}
```

```rust
pub struct LoginSuccess {
    pub user_id: Uuid,
    pub user_name: String,
    pub properties: Vec<Property>,
    /// The session id. Only on the wire since the 26.2 protocol.
    pub session_id: Option<Uuid>,
}

// decode
session_id: r.gated(version.has(Feature::LoginSuccessSessionId), Reader::uuid)?,

// encode
if version.has(Feature::LoginSuccessSessionId) {
    let session_id = self.session_id.ok_or(InternalError::MissingField { .. })?;
    w.uuid(&session_id);
}
```

Decoding a 1.21 client yields `session_id: None`; decoding a 26.2 client yields `Some(..)`. Encoding
drops the field for old clients. Handlers fill it from the feature, never from a number:

```rust
session_id: version.has(Feature::LoginSuccessSessionId).then(Uuid::new_v4),
```

Why named features rather than `version >= 775` inline:

* A backport or a re-numbered snapshot changes one line, not every codec.
* The feature name documents *what* changed; `775` documents nothing.
* Two fields that arrived together share a feature, so they cannot drift apart.

**Fail closed when encoding.** If the version requires the field and it is `None`, the encoder
returns an error instead of writing a short frame. A truncated frame desynchronises the client with
no diagnostic; a refused send is one log line pointing at the exact field. This is the one place the
driver is deliberately strict about *our* mistakes rather than the peer's.

| Pros                                                            | Cons                                                                                     |
|-----------------------------------------------------------------|------------------------------------------------------------------------------------------|
| One type for all versions; handlers mostly ignore the gate       | `Option` for a field that is mandatory on modern versions is slightly awkward to consume  |
| Version numbers confined to one table                            | Gated fields are only checked at runtime, not by the type system                          |
| Wrong combinations fail loudly at encode time                     |                                                                                          |

Because the codecs are hand-written, `Option<T>` is now a *per-packet* choice rather than a rule the
macro imposed. `LoginSuccess` is clientbound and Passage only ever encodes it, so it could equally
carry a non-optional `session_id` that older versions never see. Prefer `Option<T>` where the packet
is decoded too, and the non-optional form where the gate only affects encoding.

### B3 -- A per-field DSL / proc-macro derive

Richer declaration syntax: `#[since(..)]`, `#[until(..)]`, `#[when(feature)]`, changed types via
`#[variant(..)]`.

| Pros                                                      | Cons                                                            |
|-----------------------------------------------------------|-----------------------------------------------------------------|
| Handles removed fields and changed encodings, not just added | A proc-macro crate to maintain; worse error messages            |
| Reads like a schema                                        | Everything it can express, an `if` in the codec already expresses |

**Recommendation: B2.** B3 is where the `packet!` experiment ended up going, and the note above
records why it was reverted -- richer syntax buys nothing over a hand-written `if`, and it costs the
ability to write the case the syntax did not anticipate.

### What B2 handles that the macro could not

* **A field was removed.** `if !version.has(Feature::X) { .. }` -- the mirror of the gate above. No
  new syntax needed.
* **A field changed type.** Branch in the codec, or give the field an enum with its own `Wire` impl.
  Composite wire types are ordinary code (`Property` in `demo::packets` is one).
* **Fields were reordered.** Two `if` arms. This is the case that forced the macro's removal, because
  a generated codec always emits declaration order.
* **A packet was split or merged.** Two declarations with disjoint `ids` ranges, one handler each.
  This is the case where A1 is genuinely correct.

## Validation

Version tables are exactly the kind of data that rots. Three cheap defences:

1. **Building the router validates it.** `RouterBuilder::build(SUPPORTED_VERSIONS)` resolves every
   registered packet's ID at every supported version, so a duplicate within a phase is a `BuildError`
   at startup rather than a runtime miss on the first affected client. There is no separate
   `validate()` call to forget, because building *is* validating.
2. **Round-trip tests per packet per supported version.** Load-bearing now that codecs are written by
   hand: `demo::packets::tests::every_packet_roundtrips_in_every_version`.
3. A golden-bytes test per packet for at least the oldest and newest supported version, so a codec
   change that keeps round-tripping but changes the wire format is still caught. Not implemented.
