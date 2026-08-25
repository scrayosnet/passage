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

The state `SCRATCH.md` reached:

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
| One type per packet, versions handled inside | The mapping exists twice; `SCRATCH.md` marks this as a TODO for exactly that reason        |
| Explicit and greppable                      | The `match` grows as `packets x versions`; a missing arm is a runtime miss, not an error   |

### A3 -- Declarative table, both directions derived *(recommended)*

The packet declares its ID per version once. Encoding asks the packet; decoding asks every packet
once, at connection setup, and builds a table.

```rust
packet! {
    pub struct LoginSuccess { /* fields */ }
    phase = Login;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x02 ];   // newest first; "from this version on, this id"
}
```

```rust
// generated
fn id(version: ProtocolVersion) -> Option<i32> {
    if version.at_least(versions::V1_20_5) { return Some(0x02); }
    None
}
```

```rust
// derived once per connection, when the handshake pins the version
let bound = router.bind(version)?;      // (phase, id) -> handler
```

A packet that moved is one more line, still in one place:

```rust
ids = [
    versions::V26_2   => 0x03,   // moved
    versions::V1_20_5 => 0x02,
];
```

| Pros                                                                                  | Cons                                                                  |
|---------------------------------------------------------------------------------------|-----------------------------------------------------------------------|
| One source of truth; decode table cannot drift from the encoder                        | A macro (or later a derive) to read and debug                          |
| ID collisions are detectable: `Router::validate` over the supported range at startup    | Reverse lookup costs one table build per connection (a few hundred ns) |
| "Does not exist in this version" is representable (`None`) and enforced when sending    | Ordering of the `ids` list is a convention the macro cannot check      |
| Dispatch is a table index, not a linear `match`                                        |                                                                       |

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
packet! {
    pub struct LoginSuccess {
        pub user_id: Uuid,
        pub user_name: String,
        pub properties: Vec<Property>,
        pub session_id: Option<Uuid> = since(LoginSuccessSessionId),
    }
    phase = Login;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x02 ];
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
returns an error instead of writing a short frame:

```rust
(true, None) => Err(InternalError::MissingField { packet, field, version })
```

A truncated frame desynchronises the client with no diagnostic; a refused send is one log line
pointing at the exact field. This is the one place the driver is deliberately strict about *our*
mistakes rather than the peer's.

| Pros                                                            | Cons                                                                                     |
|-----------------------------------------------------------------|------------------------------------------------------------------------------------------|
| One type for all versions; handlers mostly ignore the gate       | `Option` for a field that is mandatory on modern versions is slightly awkward to consume  |
| Version numbers confined to one table                            | Cannot express "field changed type" -- see below                                          |
| Wrong combinations fail loudly at encode time                     | Gated fields are only checked at runtime, not by the type system                          |

### B3 -- A per-field DSL / proc-macro derive

The same as B2 with richer syntax: `#[since(..)]`, `#[until(..)]`, `#[when(feature)]`, changed types
via `#[variant(..)]`.

| Pros                                                      | Cons                                                            |
|-----------------------------------------------------------|-----------------------------------------------------------------|
| Handles removed fields and changed encodings, not just added | A proc-macro crate to maintain; worse error messages            |
| Reads like a schema                                        | Easy to over-build before the cases are known                    |

**Recommendation: B2 now, B3 when a second kind of drift actually appears.** `packet!` is a
`macro_rules!` macro today; moving it to a derive later does not change any declaration site.

### What B2 cannot do, and what to do then

* **A field was removed.** Add `= until(Feature)`, mirroring `since`. Not yet implemented.
* **A field changed type.** Give the field an enum type with its own hand-written `Wire` impl that
  branches on the version. Composite wire types are ordinary code (`Property` in `demo::packets` is
  one); only packets are generated.
* **A packet was split or merged.** Two declarations with disjoint `ids` ranges, one handler each.
  This is the case where A1 is genuinely correct.

## Validation

Version tables are exactly the kind of data that rots. Three cheap defences:

1. `Router::validate(SUPPORTED_VERSIONS)` at startup: duplicate IDs within a phase become a boot
   failure instead of a runtime miss on the first affected client.
2. Round-trip property tests per packet *per supported version* (the existing `fake`-based
   `assert_packet` helper generalises to this by taking a version).
3. A golden-bytes test per packet for at least the oldest and newest supported version, so a codec
   change that keeps round-tripping but changes the wire format is still caught.
