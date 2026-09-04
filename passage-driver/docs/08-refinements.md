# Refinements to the reference implementation

This document reviews the first implementation of the driver against the proposals in `01`–`07`.

**Status: applied.** Everything below is implemented in `passage-driver/src`, and documents `01`–`07`
have been updated to match. It is kept as-is rather than folded into them, because three of these
findings *reversed* an earlier recommendation and the reasoning that produced the reversal is worth
more than the conclusion alone. Snippets are sketches of the target shape written before the work;
where the implementation ended up somewhere different, [the outcome section](#what-actually-changed)
says so.

It is in two halves:

* [Part 1](#part-1-the-three-concerns) — the three concerns raised in review. All three were
  accepted; two of them turned out to fix ordering bugs in the code, not just to remove lines.
* [Part 2](#part-2-critical-scan) — findings from a pass over the rest of the implementation,
  ordered by value.

A [summary table](#net-effect), [what actually changed](#what-actually-changed) and the
[impact on docs 01–07](#impact-on-the-existing-documents) are at the end.

---

## Part 1: the three concerns

### R1 — Delete `packet!`, hand-write the codecs

**Verdict: accepted, and it buys more than it costs.**

The macro is 140 lines of `macro_rules!` (plus two `#[doc(hidden)]` helper macros) that saves about
ten lines per packet. That trade alone is arguable. What settles it is your second point: the macro
*structurally* welds the in-memory representation to the wire format. Three things follow from that
weld, and all three are visible in the current code.

**1. Fields must appear in declaration order, always.** A version that reorders, splits or retypes a
field cannot be expressed. Today's only escape hatch is a second packet type, which is the outcome
`02-versioning.md` argues against.

**2. Wire types leak into the domain.** Because the macro picks an encoding from a field's *type*,
`VarInt`/`VarLong` newtypes have to exist and have to appear in the struct. So `Intention.intent` is
a `VarInt`, and validating it lands in the handler:

```rust
// demo/server.rs today: the decoder accepted anything, so the handler has to parse.
let intent = match packet.intent.0 {
    1 => Intent::Status,
    2 => Intent::Login,
    3 => Intent::Transfer,
    _ => return Flow::fail(ProtocolError::UnexpectedPacket { .. }),
};
```

That is decoding, in protocol logic, reported as an `UnexpectedPacket`. Hand-written, the field is
an `Intent` and the handler never sees an invalid one.

**3. Every string shares one limit.** `impl Wire for String` reads `reader.limits().max_string_len`,
which defaults to `32_767 * 3 = 98_301` bytes. So `server_address` — a hostname — may be 98 KB, and
`user_name` too. There is no way to say otherwise at the declaration site. Hand-written decoders
name the limit per field, which is a real hardening improvement, not a stylistic one.

#### Target shape

```rust
/// The handshake, the first packet on every connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intention {
    pub protocol_version: ProtocolVersion,
    pub server_address: String,
    pub server_port: u16,
    pub intent: Intent,
}

impl Packet for Intention {
    const NAME: &'static str = "Intention";
    const PHASE: Phase = Phase::Handshake;
    const DIRECTION: Direction = Direction::Serverbound;

    // Version-independent: this has to decode before a version is known.
    fn id(_version: ProtocolVersion) -> Option<i32> {
        Some(0x00)
    }

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            protocol_version: ProtocolVersion::new(r.var_int()?),
            // The limit belongs to the field, not to a crate-wide default.
            server_address: r.string("server_address", 255)?,
            server_port: r.u16()?,
            intent: Intent::from_wire(r.var_int()?)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.var_int(self.protocol_version.get());
        w.string(&self.server_address);
        w.u16(self.server_port);
        w.var_int(self.intent.to_wire());
        Ok(())
    }
}
```

The `ids` table is the one declarative part worth keeping. It does not need a macro:

```rust
/// Resolves a packet id from a table of `(first version, id)` pairs, newest first.
pub fn ids(version: ProtocolVersion, table: &[(ProtocolVersion, i32)]) -> Option<i32> {
    table
        .iter()
        .find(|(since, _)| version.at_least(*since))
        .map(|(_, id)| *id)
}

// in the impl:
fn id(version: ProtocolVersion) -> Option<i32> {
    ids(version, &[(versions::V1_21_2, 0x03), (versions::V1_20_5, 0x02)])
}
```

A version-gated field becomes an `if`, and the fail-closed decision is written out instead of being
implied by `= since(...)`:

```rust
fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()> {
    w.uuid(&self.user_id);
    w.string(&self.user_name);
    w.array(&self.properties, version);
    if version.has(Feature::LoginSuccessSessionId) {
        // Emitting a truncated frame would desynchronise the client with nothing to diagnose it
        // from, so this is deliberately an internal error rather than a default.
        let session_id = self.session_id.ok_or(InternalError::MissingField {
            packet: Self::NAME,
            field: "session_id",
            version,
        })?;
        w.uuid(&session_id);
    }
    Ok(())
}
```

Note what this frees up: whether a gated field is `Option<T>` is now a **per-packet** choice.
`LoginSuccess` is clientbound and Passage only ever encodes it, so it could equally carry a
non-optional `session_id` that older versions simply never see. The macro forced one answer on every
packet.

#### Move `finish()` up so no decoder can forget it

The macro currently emits `reader.finish(NAME)?` at the end of every generated decoder. Hand-written
decoders could omit it — so don't leave it to them. The router's erased decoder is a single place
that runs for every packet:

```rust
// Router::on
let erased = Box::new(move |ctx: Ctx<'_, S>, payload: &[u8]| {
    let version = ctx.version();
    let mut reader = Reader::new(payload, ctx.limits());
    let packet = P::decode(&mut reader, version)?;
    reader.finish(P::NAME)?;      // once, for every packet, not once per decoder
    handler.call(ctx, packet)
});
```

This is strictly better than the macro: the check moves from "generated into N places" to "enforced
in one", so it cannot be forgotten *or* be inconsistent.

#### Shrink `Wire` to what is left

With encodings chosen by the call, not by the type, most of `wire.rs`'s trait surface is dead
weight. `VarInt`, `VarLong` and the `Wire` impls for `bool`/`u8`/`u16`/`i64`/`u64`/`Uuid`/`String`
all exist only so the macro could write `<$ty as Wire>::read`. They can go; `Reader`/`Writer`
methods already cover them. What remains worth a trait is composite values reused across packets:

```rust
/// A composite value that appears in more than one packet.
///
/// Primitives are not in here: the *call* picks the encoding (`r.var_int()`, `r.u16()`), which is
/// the choice a `Wire for i32` impl could not express anyway.
pub trait Wire: Sized {
    fn read(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self>;
    fn write(&self, w: &mut Writer<'_>, version: ProtocolVersion);
}
```

The `field: &'static str` parameter goes away with it — the caller names the field when it calls
`r.string("user_name", 16)`, so threading a label through the trait is redundant.

#### Costs and risks

* About 10–12 extra lines per packet. Passage needs roughly 25–30 packets, so ~300 lines total.
  `docs/README.md` already commits to "not a full protocol database", so this stays bounded.
* Roundtrip tests become load-bearing rather than nice-to-have, because a decoder and encoder can now
  disagree. The property is cheap to test generically:
  `assert_eq!(roundtrip(&p, version), p)` for every packet × every supported version, as a single
  table-driven test.
* If the packet count ever does reach the hundreds, the escape hatch is codegen from a protocol data
  source — which emits exactly this hand-written shape and is a much better fit than `macro_rules!`.

---

### R2 — One task set plus a read gate, instead of `Pending` vs `Detach`

**Verdict: accepted. It removes a whole file and fixes a hangup-detection gap.**

You are right that `Flow::Pending` is not really backpressure. While the driver waits for
authentication, a compliant client sends *nothing* — so "stop reading" is a **protocol assertion
dressed up as flow control**. Today the assertion is unenforced: an early packet sits in the socket
buffer and gets processed afterwards as if it had arrived on time.

There is a second, more concrete cost. Because the read arm is disabled while `pending.is_some()`,
the driver **cannot see the peer hang up** during an in-flight adapter call. A client that
disconnects mid-authentication leaves the gRPC/HTTP request running and the connection slot held
until the future resolves on its own.

Both are fixed by keeping the socket polled and rejecting instead:

```rust
pub struct Driver<S, T> {
    framed: Framed<T, FrameCodec>,
    // ...
    /// Handler work in flight, polled on this task -- no `tokio::spawn`, no `JoinError`.
    tasks: FuturesUnordered<BoxFuture<'static, Result<()>>>,
    /// How many in-flight tasks require the peer to stay quiet.
    exclusive: usize,
}
```

```rust
let step = tokio::select! {
    biased;

    // 1. Everything handlers asked for, before anything else.
    op = self.ops.recv() => Step::Op(op),

    // 2. Handler work, exclusive or not -- one set, one arm.
    done = self.tasks.next(), if !self.tasks.is_empty() => Step::Task(done),

    // 3. Cancellation and deadlines.
    () = self.shutdown.cancelled() => Step::Shutdown,
    () = self.deadlines.next(), if self.deadlines.armed() => Step::Expired,

    // 4. Ticks.
    _ = tick(&mut self.ticker), if self.ticker.is_some() => Step::Tick,

    // 5. Input. Polled even while gated: a frame arriving now is a protocol break, and an EOF
    //    is a hangup we want to act on immediately rather than after the adapter call returns.
    frame = self.framed.next() => Step::Frame(frame),
};
```

```rust
fn handle_frame(&mut self, frame: Frame) -> Result<()> {
    if self.exclusive > 0 {
        // The peer was told to wait. Whatever it sent, it sent too early.
        return Err(ProtocolError::EarlyPacket {
            phase: self.phase,
            id: frame.id,
        }
        .into());
    }
    // ... unchanged
}
```

`Framed::next()` is cancel-safe (the partial frame lives in the codec, not in the future), so
keeping this arm enabled every iteration costs nothing.

#### Exclusivity should be a property of the task, not a manual flag

You suggested a flag. A flag works, but it can leak: whoever closes it must remember to open it, and
a missed reopen stalls the connection. Tying it to the task removes that failure mode entirely — the
driver increments on spawn and decrements on completion, so there is nothing to forget:

```rust
impl<S> ConnHandle<S> {
    /// Runs a future alongside the connection. Further packets keep being dispatched.
    pub fn spawn(&self, f: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.queue(Op::Spawn { future: Box::pin(f), exclusive: false })
    }

    /// Runs a future and requires the peer to stay quiet until it resolves.
    ///
    /// This is what `Flow::Pending` was: no further packet is dispatched, no state is observed
    /// half-updated. The difference is that an early packet is now *reported* instead of buffered.
    pub fn exclusive(&self, f: impl Future<Output = Result<()>> + Send + 'static) -> Result<()> {
        self.queue(Op::Spawn { future: Box::pin(f), exclusive: true })
    }
}
```

The soundness of this rests on the op queue's priority: the handler returns, its `Op::Spawn` is
drained *before* the read arm is reached, so `exclusive` is already non-zero by the time the next
frame — buffered or not — is looked at. Worth writing down in `04-runtime.md`, since it is
load-bearing and not obvious.

#### Handlers become plain synchronous functions

With `spawn`/`exclusive` as the only async mechanism, `Flow` has nothing left to express. Handlers
return `Result<()>`:

```rust
type Handler<S, P> = dyn Fn(Ctx<'_, S>, P) -> Result<()> + Send + Sync;
```

```rust
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Result<()> {
    let conn = ctx.conn.clone();
    let version = ctx.version();

    ctx.conn.exclusive(async move {
        let (name, id) = authenticate(&packet.user_name).await?;

        // Both are queued, so they land in exactly this order: state first, then the wire.
        let profile = (name.clone(), id);
        conn.update(move |s: &mut Session| s.profile = Some(profile))?;
        conn.send(&LoginSuccess {
            user_id: id,
            user_name: name,
            properties: Vec::new(),
            session_id: version.has(Feature::LoginSuccessSessionId).then(Uuid::new_v4),
        })
    })
}

fn on_login_acknowledged(ctx: Ctx<'_, Session>, _: LoginAcknowledged) -> Result<()> {
    ctx.conn.set_phase(Phase::Configuration)?;
    let conn = ctx.conn.clone();
    // Not exclusive: keep-alives have to keep flowing while backend selection runs.
    ctx.conn.spawn(async move {
        let target = select_backend().await?;
        conn.send(&Transfer { host: target.host, port: target.port })?;
        conn.close()
    })
}
```

This is also a better answer to the original motivation for `Flow`. The argument was "don't box a
state machine for a handler that completes synchronously". Under `Flow`, a sync handler still
returned an enum. Now it returns a `Result<()>` with no wrapper at all, and the box appears only
where a future is genuinely spawned. `#[must_use]` comes free from `Result`.

#### What this deletes

`src/flow.rs` in full (125 lines) — `Flow`, `Outcome`, `Update`, and their constructors. From the
driver: the `pending` field, `Step::Pending`, `accept_flow` and its `debug_assert`, `handle_joined`,
the `JoinSet`, `InternalError::Handler(JoinError)`, and the `if self.pending.is_none()` guards on
both the tick and read arms.

#### Risks

* **Strictness.** A client that pipelines during the exclusive window is now disconnected where it
  used to be tolerated. Vanilla waits at every point where Passage needs to (`EncryptionResponse` →
  verify → `LoginSuccess` → `LoginAcknowledged`), so this should be safe — but it is a behaviour
  change against unknown clients. If it needs a fallback, "defer instead of reject" is a config
  field and a select condition, not a second code path:

  ```rust
  frame = self.framed.next(), if self.exclusive == 0 || self.config.reject_early => ...
  ```

* **`FuturesUnordered` vs `JoinSet`.** Polling on the connection task removes `tokio::spawn`, the
  `JoinError` handling and the panic-to-internal-error conversion. The trade is that a panicking
  handler now takes the connection task down instead of being caught — which whoever spawned the
  connection sees anyway. Keep `JoinSet` if per-task panic isolation is worth the extra arm; nothing
  else in this design changes.

* **A task that never resolves** stalls dispatch. That is what makes the connection deadline
  ([F4](#f4--the-driver-has-no-deadline)) mandatory rather than a nice-to-have.

---

### R3 — `Op::With` instead of `Update` returned once at the end

**Verdict: accepted, and it fixes an ordering bug.**

`Update<S>` can only be applied when the whole future has resolved. That is not just a limitation —
it makes state and wire observably inconsistent. A detached task sends its packets *during* the
future and commits its state *after* it, so anything that runs in between sees a session whose
outcome has been announced to the client but not recorded locally. The tick handler is exactly such
a thing. Under R2 every async handler is a task, so this stops being a corner case.

Making it an operation puts state changes in the same ordered queue as everything else:

```rust
pub enum Op<S> {
    Send(Encoded),
    Encrypt(Box<dyn Cipher>),
    SetPhase(Phase),
    /// Run a closure against the connection state, in queue order.
    With(Box<dyn FnOnce(&mut S) + Send>),
    Spawn { future: BoxFuture<'static, Result<()>>, exclusive: bool },
    Flush(oneshot::Sender<()>),
    Close,
}
```

The driver's arm is one line, replacing `Update::run`, the apply in `Step::Pending`, and the apply in
`handle_joined`:

```rust
Op::With(f) => f(&mut self.state),
```

#### One op covers both writing and reading

Your oneshot back-channel generalises nicely: if the closure can return a value, the same operation
gives detached tasks the ability to **read** state, which today they cannot do at all.

```rust
impl<S> ConnHandle<S> {
    /// Queues a change to the connection state, ordered with everything else queued.
    pub fn update(&self, f: impl FnOnce(&mut S) + Send + 'static) -> Result<()> {
        self.queue(Op::With(Box::new(f)))
    }

    /// Runs `f` against the connection state and returns its value.
    ///
    /// Resolves once the driver has drained everything queued before it, so this doubles as a
    /// barrier: `conn.with(|_| ()).await` means "everything I queued has been carried out".
    pub async fn with<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut S) -> R + Send + 'static,
    ) -> Result<R> {
        let (tx, rx) = oneshot::channel();
        self.queue(Op::With(Box::new(move |state| {
            // The receiver having gone away only means nobody is listening anymore.
            let _ = tx.send(f(state));
        })))?;
        rx.await.map_err(|_| Error::Closed)
    }
}
```

Two rules to document, both consequences of the closure running inside the driver's drain loop with
`&mut S`:

* It must not block or await. It may queue further operations through a cloned handle.
* If the connection ends first, the sender is dropped and `with` resolves to `Error::Closed`. That
  is the same contract `flush` already has.

---

## Part 2: critical scan

### F1 — A dispatch table is built per connection

`NOTES.md` already flags this, and it is the most expensive thing in the current code.
`Driver::new` calls `router.bind(initial_version)`, which allocates `Phase::COUNT` vectors and clones
an `Arc` per registered packet — then the handshake handler changes the version and `handle_frame`
does it all again. Two full table builds per connection, on a port that scanners hit.

The fix also consolidates three other things. Build the tables once, at startup, for the supported
version range:

```rust
/// A router with its dispatch tables built. Built once, shared by every connection.
pub struct Router<S> {
    inbound: Direction,
    unknown: UnknownPolicy,
    entries: Box<[Entry<S>]>,
    tables: HashMap<ProtocolVersion, Arc<Bound<S>>>,
    /// Used for any version without a table: the version-independent packets, which is enough to
    /// answer a status ping and refuse a login.
    fallback: Arc<Bound<S>>,
    tick: Option<Arc<dyn TickHandler<S>>>,
}

impl<S> RouterBuilder<S> {
    /// Builds the tables for `versions`, failing on id collisions and direction mistakes.
    pub fn build(
        self,
        versions: impl IntoIterator<Item = ProtocolVersion>,
    ) -> Result<Router<S>, BuildError> { /* ... */ }
}

impl<S> Router<S> {
    fn table(&self, version: ProtocolVersion) -> &Arc<Bound<S>> {
        self.tables.get(&version).unwrap_or(&self.fallback)
    }
}
```

What falls out:

* **`Router::validate` disappears** — building *is* validating. Today `validate` is opt-in, and
  `tests/flow.rs:372` shows it has to be called by hand; an id collision in a router nobody
  validated is a runtime error on the first client that hits the packet.
* **`Router::on`'s `assert_eq!` on direction becomes a `BuildError`**, consistent with the rest.
* **`Driver::new` becomes infallible** ([F2](#f2--drivernew-only-returns-result-because-bind-can-fail)).
* **A hostile version cannot cost memory.** The version space is `i32`; only supported versions get a
  table, everything else shares the fallback. Building the fallback at `ProtocolVersion::UNKNOWN`
  yields precisely the packets whose id table starts there — handshake and status — which is the
  behaviour you want for an old client asking what to install.

Optionally, dedupe identical tables behind the `Arc`: adjacent protocol versions usually have the
same id map, so twenty versions typically need three tables. Worth a footnote, not worth doing first.

### F2 — `Driver::new` only returns `Result` because `bind` can fail

Nothing else in `new` fails. With F1 the signature becomes

```rust
pub fn new(io: T, router: Arc<Router<S>>, state: S, config: DriverConfig, shutdown: CancellationToken)
    -> (Self, ConnHandle<S>)
```

which removes an error path from every accept loop and the `.expect("router binds")` calls in the
tests.

### F3 — A handler cannot classify its own errors

`05-errors-and-hardening.md` makes blame the organising principle, but the only channel a handler has
is `InternalError::Handler(Box<dyn Error>)` — which is hard-wired to `Class::Internal`. The demo shows
what that costs:

```rust
// demo/server.rs:211 -- a client that stops answering keep-alives
return Flow::fail(Error::Internal(InternalError::Handler(
    "client missed a keep-alive".into(),
)));
```

A timeout is ordinary client behaviour. Classified `Internal`, it is logged at `warn` and reported to
Sentry — the taxonomy's whole purpose, inverted, in the reference implementation of it. Let the layer
above supply the blame and the label:

```rust
pub enum Error {
    Protocol(ProtocolError),
    Transport(std::io::Error),
    Internal(InternalError),
    /// Raised by the layer above. It supplies the class and the metric label, because the driver
    /// cannot know whether a failed authentication is the peer's fault, ours, or Mojang's.
    Handler {
        class: Class,
        /// Low-cardinality and never peer-controlled, like every other label.
        label: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    Closed,
}
```

`InternalError::Handler`'s other two uses go away on their own: the `JoinError` case disappears with
R2, and the router's `format!(...).into()` — a `String` allocated into a boxed error on a startup
path — becomes a `BuildError` variant under F1. `Error` ends up smaller *and* more expressive.

### F4 — The driver has no deadline

`DriverConfig` has `tick_interval` and nothing else. A peer that connects and never sends a byte holds
a task and a socket indefinitely; the demo's tick only acts in the configuration phase.
`04-runtime.md:169` and `05-errors-and-hardening.md:170` both note this is unimplemented — R2 promotes
it from "should have" to "required", because it is the backstop for a task that never resolves.

```rust
pub struct DriverConfig {
    pub limits: Limits,
    pub tick_interval: Option<Duration>,
    /// Hard cap on the whole connection. Passage connections are short by construction: a status
    /// ping is two packets, a login is a handful.
    pub max_lifetime: Option<Duration>,
    /// How long the peer may be silent before the connection is dropped.
    pub max_idle: Option<Duration>,
    pub initial_version: ProtocolVersion,
    pub initial_phase: Phase,
}
```

with a matching `Completion::TimedOut`. This belongs in the driver rather than in the caller: it owns
the clock and the socket, and wrapping `driver.run()` in `tokio::time::timeout` drops the future
mid-flight, so `finish()` never runs and in-flight tasks are not cancelled cleanly.

### F5 — An `Arc` clone and two extra indirections per dispatch

`Bound::lookup` returns `Option<Arc<Entry<S>>>` and clones on **every packet**, purely to avoid
holding a borrow of `self.bound` while `ctx` holds `&mut self.state`. Rust splits those borrows
happily — `Ctx::new(&mut self.state, &self.handle, ..)` already borrows two fields at once — so the
clone is unnecessary. With F1's flat entry list the table becomes an index:

```rust
pub struct Bound<S> {
    /// `phase -> id -> index into the router's entries`, so a lookup is two loads and no refcount.
    tables: [Box<[Option<u16>]>; Phase::COUNT],
    _state: PhantomData<S>,
}
```

Two smaller allocations in the same area:

* `Router::on` does `let handler = Arc::new(handler)` and then moves it into the erased closure. The
  closure is the only owner; it can hold the handler by value and the `Arc` disappears.
* `Erased<S>` is a `Box<dyn Fn>` wrapping an `Arc<dyn Handler>` — two allocations per registration
  and two hops per dispatch, one of which the point above removes.

### F6 — The outbound path has the unchecked casts the inbound path forbids

`wire.rs` opens with "never let peer input reach an arithmetic operation that can panic", and the
reader honours it. The writer does not:

* `FrameCodec::encode`: `writer.var_int(item.bytes.len() as i32)` — no check against
  `limits.max_frame_len`, and a wrap at 2 GiB.
* `Writer::bytes`: `self.var_int(value.len() as i32)`.
* `Wire for Vec<T>`: `writer.var_int(self.len() as i32)`.

None are reachable today, because everything Passage sends it built itself. But "unreachable given
current callers" is not the standard the module sets, and the failure mode is a length prefix that
disagrees with the payload — the hardest kind of protocol bug to diagnose from the other end.

```rust
// FrameCodec::encode
let length = i32::try_from(item.bytes.len())
    .ok()
    .filter(|_| item.bytes.len() <= self.limits.max_frame_len)
    .ok_or(InternalError::OversizedFrame { packet: item.name, length: item.bytes.len() })?;
writer.var_int(length);
```

### F7 — The phase atomic hides a silent fallback

`ConnHandle::phase()` reads an `AtomicU8` and does
`Phase::from_index(index).unwrap_or(Phase::Handshake)` — an impossible case, silently defaulted to
the most permissive phase.

More interestingly, the phase is the one piece of connection state that changes *repeatedly and
mid-stream*, and it is applied eagerly while packet sends are queued. That is the same ordering
hazard R3 fixes for `S`, and it is why `set_phase` carries an invariant comment
(`conn.rs:124–129`) telling callers when they may call it. As an operation, the invariant is
structural instead of documented:

```rust
Op::SetPhase(phase) => self.phase = phase,
```

The phase then lives in the driver, `Ctx::phase()` reads the driver's copy (always current), and
`Shared` loses one atomic plus the silent fallback. `Phase::index`/`COUNT` stay — the dispatch tables
need them.

Keep `version` as an atomic: `ConnHandle::send` needs it to resolve the outbound id, and it is set
exactly once, before anything is queued.

### F8 — `Step::Op(None)` is unreachable

`ops.recv()` returns `None` only once every sender is dropped, and the driver holds `self.handle`,
which holds one. So `Step::Op(None) => break Completion::Closed` never fires. Harmless, but it
reads as a second way for a connection to close, which is exactly the confusion
`05-errors-and-hardening.md` sets out to remove.

### F9 — Dead and misplaced code

* **`src/hooks.rs` is dead.** It is not in `lib.rs`'s module list (`grep -rn "mod hooks"` finds
  nothing) and predates the current design — `DriverError`, `Flow<'_, E>`, `Arc<Mutex<S>>`, a
  method-per-packet trait. Delete it; `docs/03-dispatch.md` already records the argument against
  that shape.
* **`pub mod demo` ships into the library.** The demo server and its packet set are compiled into
  every consumer of `passage-driver`. Gate it behind a feature, or move it to `examples/` with the
  packet set in `tests/`.
* **`Session::keep_alive_misses`** is incremented and then immediately fails the connection, so it is
  never read.
* **`SCRATCH.md` and `NOTES.md`** are working notes; fold what survives into `docs/` and drop them.
  (`NOTES.md` is currently the only staged change on the branch.)
* **`log_completion`** is logging policy living in a library. It belongs in `passage`, where the
  observability wiring is.

### F10 — A wrong-phase packet gets a misleading diagnosis

Tables are per phase, so a packet sent in the wrong phase does not "miss" — it collides with whatever
holds that id in the current phase. A `LoginStart` (id `0x00`, Login) arriving during Status is
decoded as a `StatusRequest` and fails as `TrailingBytes`. Not a safety problem, but the report
points at the wrong thing.

`ProtocolError::UnexpectedPacket { packet, phase }` already exists for this and is currently only
constructed by the demo for an invalid intent value — which is a decode failure, not a phase problem
(and goes away with R1). Giving `Bound` an `id -> name` index across all phases lets `dispatch`
answer the question properly, and gives the variant its intended user.

### F11 — Every packet costs a write syscall

`handle_op(Op::Send)` uses `SinkExt::send`, which is `feed` **plus** `flush`. So a handler that
queues a `LoginSuccess` and a `KeepAlive` back to back produces two writes where one would do. Since
the driver already knows when it has run out of work, batching is nearly free:

```rust
Op::Send(encoded) => self.framed.feed(encoded).await?,   // buffer only
// ...
// after the op queue drains and before blocking on anything else:
if self.framed.write_buffer().has_remaining() {
    self.framed.flush().await?;
}
```

This also gives `Op::Flush` a reason to exist. Today it is redundant — `send` already flushes, so
`conn.with(|_| ()).await` from R3 would be an equivalent barrier.

### F12 — `tick_interval` and the tick handler are configured independently

`DriverConfig::tick_interval` arms the timer; `Router::on_tick` provides the handler. Set the first
without the second and the driver wakes on a timer to do `handle_tick`'s early `return Ok(())`. Take
the interval from the router (which knows whether it has a handler), or reject the mismatch at build
time.

---

## Net effect

| Change | Removes | Adds |
|---|---|---|
| R1 — hand-written codecs | `packet!` + 2 helper macros (~140 lines), `VarInt`/`VarLong`, 7 `Wire` primitive impls, `read_since`/`write_since` (~110 lines of `wire.rs`) | ~10 lines per packet; an `ids()` helper; roundtrip tests as a hard requirement |
| R2 — one task set + gate | `src/flow.rs` (125 lines), `Driver::pending`, `accept_flow`, `handle_joined`, `JoinSet`, two select guards | `exclusive: usize`, one select arm, `ProtocolError::EarlyPacket` |
| R3 — `Op::With` | `Update<S>` and its impls, three separate apply sites | one `Op` variant, one driver arm, `ConnHandle::{update, with}` |
| F1 — build tables once | `Router::validate`, `bind`'s per-connection allocation, `Driver::new`'s `Result` | `RouterBuilder`, `BuildError`, a version list at startup |
| F3 — classified handler errors | `InternalError::Handler`'s three unrelated uses | `Error::Handler { class, label, source }` |
| F4 — deadlines | per-caller `tokio::time::timeout` wrappers | two `DriverConfig` fields, `Completion::TimedOut` |

The handler signature at the end of it:

```rust
// before
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Flow<Result<Update<Session>>>

// after
fn on_login_start(ctx: Ctx<'_, Session>, packet: LoginStart) -> Result<()>
```

## What actually changed

The implementation followed the plan above with one significant extension and a handful of
consequences that only became visible once the code was written.

### The extension: `Ctx` gives up `&mut S` entirely

R3 as proposed left `Ctx::state` as `&mut S` for synchronous handlers and added `Op::With` for
asynchronous ones. The rule adopted instead is stricter and simpler to state: **every interaction
between a handler and the connection is an `Op`.** So `Ctx` carries `&S` — reads are free — and
`send`, `update`, `set_phase`, `set_version`, `encrypt`, `spawn`, `exclusive` and `close` are all
queued.

That turned out to pay for itself three times over:

* **`set_version` and `set_phase` are ordered.** Both were eagerly-applied atomics, and `set_phase`
  carried a documented invariant telling callers when they were allowed to call it. As operations the
  invariant is structural, and `Shared` lost both atomics — including
  `Phase::from_index(..).unwrap_or(Phase::Handshake)`, a silent fallback for an impossible case
  ([F7](#f7--the-phase-atomic-hides-a-silent-fallback)).
* **`Op::Send` carries the packet, not bytes.** Since the driver owns the version, it encodes at
  drain time. A handler no longer needs the version to send anything, and a task that outlives its
  handler cannot encode against a stale one. The ordering property is unchanged, because the encode
  happens *at* drain time.
* **A handler cannot half-apply itself.** One that queues two operations and then returns an error
  has applied neither. Under `&mut S` it would have applied the first.

The cost is that `ctx.state` is a snapshot: a handler cannot observe its own effects. In practice
that reads better, not worse — `on_keep_alive_response` now does its comparison *inside* the update,
against the current value, rather than against one read a moment earlier.

`AnyPacket` is the only new abstraction it required: an object-safe view of `Packet`, implemented on
a private `Erased<P>` wrapper rather than blanket-implemented, so no packet type ever has two
`encode` methods to disambiguate between.

### Consequences that only showed up in the code

* **Ticks must be suppressed while the gate is shut.** Not in the proposal, and necessary: a
  keep-alive sent into an exclusive window would invite exactly the response the gate rejects. The
  old design disabled ticks while a handler was pending for a different reason, so the guard survived
  with a new justification.
* **`Limits` lost two of its four fields.** Once every field named its own bound, nothing read
  `max_string_len` or `max_array_len` — the ~98 KB default existed only because a generated codec had
  nowhere else to look. They are deleted rather than deprecated; `max_frame_len` bounds every field
  transitively.
* **`log_completion` moved out of the driver.** [F9](#f9--dead-and-misplaced-code) called it
  misplaced; it now lives in `demo::server`, which is where the "this is the caller's policy" claim
  can actually be demonstrated.
* **`Op::Encrypt` needs no flush.** Worth recording because the opposite looks true: the codec only
  encrypts what it encodes from the switch onward, so bytes already in the write buffer stay
  plaintext. Batched flushing ([F11](#f11--every-packet-costs-a-write-syscall)) therefore did not
  break the switchover.
* **`FuturesUnordered` over `JoinSet`**, as proposed — which also let three tokio/tokio-util feature
  flags be dropped, since the driver no longer needs a runtime handle to spawn onto.
* **F10 is only half-fixable.** A packet whose ID is *taken* in the current phase (`LoginStart` 0x00
  in Login vs `StatusRequest` 0x00 in Status) cannot be distinguished by any dispatch design. What is
  fixed is the case where the ID is free: dispatch looks it up in the other phases and reports
  `UnexpectedPacket` by name. Recorded honestly in `03-dispatch.md` rather than claimed as solved.

### Net result

43 tests pass (25 unit, 18 end-to-end), `cargo clippy --all-targets --all-features` is clean, and
`cargo build --no-default-features` confirms the demo is genuinely optional. `src/flow.rs` (125
lines) and `src/hooks.rs` (86 lines of dead code) are gone, along with `SCRATCH.md` and `NOTES.md`,
whose surviving content is in these documents.

Five tests are new and exist because of this review: the two halves of the read gate
(`a_packet_sent_during_an_exclusive_task_is_a_protocol_error`,
`a_hangup_during_an_exclusive_task_ends_the_connection_at_once`), the state-versus-wire ordering
property (`state_is_recorded_before_the_packet_that_announces_it`), and both deadlines.

## Impact on the existing documents

All applied. `docs/README.md`'s decision table changed in three rows, and the reversals are recorded
rather than quietly edited — the reasoning that led to the original choice is still worth reading.

| Doc | Change |
|---|---|
| `02-versioning.md` | Decision 1 flipped from "declarative table (`packet!`)" to "declarative ID table + hand-written codecs". The version-gating argument (named `Feature`s, one type per packet, fail closed on a missing required field) is unaffected — only its mechanism changed. A "tried and removed" note records what the macro could not express. |
| `03-dispatch.md` | Typed registration survives unchanged. Updated for `RouterBuilder::build`, the trailing-bytes check moving into the erased decoder, and the honest limit on wrong-phase diagnostics. |
| `04-runtime.md` | Largest revision. The `Flow` section became the gate model, with the op-queue-priority argument that makes `exclusive` sound and the hangup-detection gain; state access is now a three-option comparison ending at `Op::With`; deadlines moved from "not implemented" to specified. |
| `05-errors-and-hardening.md` | Added `Error::Handler` and the classification rule, `BuildError`, `Completion::TimedOut`, the outbound-cast rule (F6), and the `Limits` shrink. |
| `06-layering-and-telemetry.md` | `log_completion` is named as caller policy; `handler.async` became `handler.task`/`exclusive`; two new metric labels worth watching after deployment. |
| `07-reference-implementation.md` | Rewritten against the final shape, with every "what this proves" row pointing at a test that exists. |
