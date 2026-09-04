# 3. Dispatch: how a packet reaches protocol logic

The requirement is "hooks and handlers instead of one big method, following Ktor / Axum / Tonic". All
four options below satisfy that literally; they differ in what happens when the protocol grows.

## C1 -- One trait with a method per packet

The shape the first sketch of this crate had (`src/hooks.rs`, since deleted):

```rust
pub trait Hooks<S> {
    fn on_tick(&mut self, ctx: &Ctx<S>) -> Flow<'_, Result<(), DriverError>>;
    fn on_handshake_intention(&self, ctx: &Ctx<S>, packet: ()) -> Flow<'_, Result<(), DriverError>>;
    fn on_status_status_request(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    fn on_configuration_login_finished(&self, ctx: &Ctx<S>, packet: &()) -> Flow<'_, Result<(), DriverError>>;
    // ... one per packet, per phase
}
```

| Pros                                                      | Cons                                                                                          |
|-----------------------------------------------------------|-----------------------------------------------------------------------------------------------|
| Static dispatch, no boxing, no allocation                  | **Every new packet is a breaking change** for every implementor, even ones that do not care     |
| Compiler-checked exhaustiveness                            | The trait grows to ~60 methods for full coverage; a client and a server implement disjoint halves and stub the rest |
| Easy to discover: the trait *is* the documentation          | No composition: no middleware, no "reuse the standard status handler and override login"        |
|                                                            | Method names encode phase + name, so a moved packet renames a method                            |
|                                                            | Cannot register two handlers, or none, for a packet                                             |

This is the shape a "backbone library" suffers from most: the library cannot add a packet without a
major version bump, so it will lag behind the protocol exactly when it should not.

## C2 -- Visitor

The early `SCRATCH.md` sketch explored this:

```rust
pub trait PacketVisitor<T: Packet> {
    fn on(&mut self, packet: &T) -> Result<(), Error> { Ok(()) }
}

pub trait PacketRegistryVisitor = PacketVisitor<PacketA> + PacketVisitor<PacketB>;
```

| Pros                                                        | Cons                                                                          |
|-------------------------------------------------------------|-------------------------------------------------------------------------------|
| Static dispatch; one impl per packet type instead of one giant trait | Requires trait aliases (unstable) or a hand-maintained super-trait bound list  |
| Default methods make "ignore this packet" free               | The bound list is the same maintenance burden as C1, moved to a `where` clause  |
| Sender does not need to know the receiver's type             | Adding a packet still changes the bound, and inference errors get long fast     |

The version dimension makes it worse: the visitor's dispatch function still needs the
`(version, id)` match from A2, so the duplication of [02-versioning.md](02-versioning.md) remains.

## C3 -- Typed registration into an erased router *(recommended)*

Registration is generic; storage is erased. This is the Axum/Tonic shape.

```rust
pub fn router() -> Result<Router<Session>, BuildError> {
    Router::builder(Direction::Serverbound)
        .unknown(UnknownPolicy::Reject)
        .on::<Intention, _>(on_intention)
        .on::<StatusRequest, _>(on_status_request)
        .on::<PingRequest, _>(on_ping_request)
        .on::<LoginStart, _>(on_login_start)
        .on::<LoginAcknowledged, _>(on_login_acknowledged)
        .on::<KeepAliveResponse, _>(on_keep_alive_response)
        .on_tick(on_tick)
        .build(SUPPORTED_VERSIONS.iter().copied())
}

fn on_ping_request(ctx: Ctx<'_, Session>, packet: PingRequest) -> Result<()> {
    ctx.send(PongResponse { payload: packet.payload })?;
    ctx.close()
}
```

`on::<P, _>` stores a closure that decodes `P` and calls the handler, so the decode and the handler
that consumes it are created together and cannot disagree. It is also the one place the
trailing-bytes check runs, which is why no hand-written decoder has to remember it:

```rust
let dispatch: Dispatcher<S> = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
    let version = ctx.version();
    let mut reader = Reader::new(payload, ctx.limits());
    let packet = P::decode(&mut reader, version)?;
    reader.finish(P::NAME)?;
    handler.call(ctx, packet)
});
```

| Pros                                                                                | Cons                                                                                     |
|-------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------|
| Adding a packet changes no trait, so it breaks nothing                               | One `Box<dyn Fn>` call per packet (irrelevant next to a syscall, but not zero)             |
| Handlers are plain functions: unit-testable without a connection                     | "Packet has no handler" is a runtime policy, not a compile error                           |
| The router *is* the protocol surface, readable in one screen                          | Closures need explicit argument types to satisfy the higher-ranked `Fn` bound (see below)   |
| Composable: build a base router and override or extend it per route                  | The type parameter `S` propagates through `Router`, `Ctx` and `ConnHandle`                  |
| Direction and phase come from the packet, so a mis-registration is a `BuildError`    |                                                                                            |

### Building is validating

`RouterBuilder::build(versions)` is the only fallible step, and it is the *only* place a router can be
wrong. It resolves every packet's ID at every supported version, so an ID collision, an out-of-range
ID and a packet registered on a router travelling the wrong way are all startup failures. Two things
follow:

* `Driver::new` is infallible. Nothing about dispatch can go wrong once a connection is running.
* A connection allocates no table. It takes an `Arc` of the one built for its version -- the earlier
  design rebuilt five vectors per connection, twice (once in `Driver::new`, once when the handshake
  changed the version).

A version with no table -- anything outside the supported range, including the garbage a scanner
sends -- gets the version-independent table, which is exactly the packets whose ID table starts at
`ProtocolVersion::UNKNOWN`. That is enough to answer a status ping and refuse a login, and it means
the `i32` version space cannot cost memory.

The runtime-policy downside is mitigated by making it explicit and strict by default:

```rust
pub enum UnknownPolicy {
    Reject,   // default: a server that needs 15 packets should refuse the 16th
    Ignore,   // for permissive clients and proxies
}
```

**Ergonomics caveat found while implementing this:** a closure passed to `on` must annotate its
arguments, because the blanket impl needs `Fn` for *any* lifetime:

```rust
// fails: "implementation of `FnOnce` is not general enough"
.on::<StatusRequest, _>(|ctx, packet| Ok(()))

// works
.on::<StatusRequest, _>(|ctx: Ctx<'_, Session>, packet: StatusRequest| Ok(()))
```

Named handler functions -- which is what a real flow uses anyway -- have no such problem. Worth
documenting rather than designing around.

## C4 -- Typestate phases

Encode the phase in the type, so a login handler cannot run in the status phase:

```rust
struct Connection<P: Phase> { /* ... */ }
impl Connection<Login> {
    fn on_login_start(self, packet: LoginStart) -> Connection<Configuration> { /* ... */ }
}
```

| Pros                                                     | Cons                                                                                    |
|----------------------------------------------------------|-----------------------------------------------------------------------------------------|
| Illegal transitions do not compile                        | The configuration phase is deliberately not lock-step: several packets in any order, repeated |
| Self-documenting flow                                     | Every transition consumes and rebuilds the connection, which fights the driver owning the socket |
|                                                          | Generic explosion once adapters are also type parameters (the current `Routes<Stat, Disc, Auth, Loca>` already shows this) |

**Recommendation: C3**, with the *runtime* half of C4 kept: the driver tracks a `Phase`, dispatch is
keyed by it, and a packet from the wrong phase is a `Peer` error rather than a mis-dispatch. That
gets the safety property without the type gymnastics.

One honest limit: because IDs are only unique *within* a phase, a packet from another phase whose ID
happens to be taken in the current one is decoded as the resident packet -- `LoginStart` is `0x00` in
Login and `StatusRequest` is `0x00` in Status, and nothing can tell them apart. That case surfaces as
`TrailingBytes` or a decode error, which is correct but uninformative, and no dispatch design fixes
it. What *is* fixed is the case where the ID is free in the current phase: dispatch looks it up in
the others and reports `UnexpectedPacket { packet, expected, phase }` by name, rather than sending
whoever reads the log looking for a missing packet definition.

## The tension worth naming: scripted vs. event-driven

The current `listen()` is a script:

```rust
let handshake = self.next_packet().await?;   // "now expect this"
let login     = self.next_packet().await?;
```

Event handlers are the opposite: the protocol pushes, you react. For a lock-step phase like login,
scripts are strictly easier to read, and a naive event model turns the flow into a state enum with
one variant per step -- a real regression in clarity.

Two ways out:

### Option 1 -- Add an `expect` primitive

Let a handler await a specific packet, registering a one-shot interest that dispatch checks first:

```rust
let response = ctx.recv::<CookieResponse>().await?;   // hypothetical
```

| Pros                                                | Cons                                                                          |
|-----------------------------------------------------|-------------------------------------------------------------------------------|
| Lock-step flows read exactly like today's code       | Two dispatch mechanisms; "who gets this packet" now depends on runtime state    |
|                                                     | A registered interest that never arrives is a leak, so it needs its own timeout |
|                                                     | Reentrancy: the handler is suspended inside dispatch while dispatch continues   |

### Option 2 -- One mechanism, and the peer is told to wait *(chosen)*

A handler that has to wait for something external hands the work to `ctx.exclusive(..)`, which says
"the peer has nothing to send until this resolves". The flow stays as sequential as it needs to be,
without a second dispatch path:

```rust
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Result<()> {
    let conn = ctx.conn.clone();
    ctx.exclusive(async move {
        let (name, id) = authenticate(&packet.user_name).await?;   // takes as long as it takes
        conn.update(move |s: &mut Session| s.profile = Some((name, id)))?;
        conn.send(LoginSuccess { /* ... */ })
    })
}
```

| Pros                                                                        | Cons                                                                    |
|-----------------------------------------------------------------------------|-------------------------------------------------------------------------|
| One dispatch path; no interest registry, no reentrancy                       | A multi-round-trip exchange becomes several handlers plus session state  |
| Ordering guaranteed: no packet is dispatched into a half-finished transition  | The "script" is split across functions rather than being one function    |
| An early packet is *reported*, not buffered and replayed                     | Stricter than the protocol strictly requires (see [04-runtime.md](04-runtime.md#the-read-gate)) |

For Passage specifically, the multi-round-trip parts (cookie request/response, encryption
request/response) are two handlers and one `Option` in the session each -- an acceptable price for
having only one mechanism. This is proven by
`tests/flow.rs::a_packet_sent_during_an_exclusive_task_is_a_protocol_error` and
`::the_login_flow_completes_when_the_client_waits_its_turn`.

## Middleware

Not implemented, and worth resisting until there is a second user. If it becomes necessary, the
natural seam is a `Layer` around the erased handler:

```rust
type Dispatcher<S> = Box<dyn for<'c> Fn(Ctx<'c, S>, &[u8]) -> Result<()> + Send + Sync>;
```

Everything a `tower::Layer` would do (tracing spans, metrics, rate limits, per-phase deadlines) can
wrap that. Note that the driver already emits the span and metrics, which is the part a layer would
otherwise be needed for.
