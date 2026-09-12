# Review: `router.rs` and `server.rs`

A close read of the two modules that assemble everything else, looking for code that is duplicated,
dead, or more complicated than the job needs. Two directions from the last round are folded in as
sections of their own: **construction should use builders** (§D) and **types should carry their
parameters rather than erasing to boxes** (§E). The second one turns out to be right in some places
and wrong in others, and the line between them is worth stating outright.

Everything else — `conn`, `wire`, `codec`, `packet` — is out of scope except where these two reach
into it. Line numbers are current as of this commit.

At a glance:

| | Finding | Cost to fix |
|---|---|---|
| A1 | `breakpoints()` is computed twice per build | one line |
| A2 | `RouterDispatcher::router()` has no callers | delete |
| A3 | Three handler traits are the same construction three times | ~46 lines removed, one API decision |
| A4 | `Entry::id` wraps a free function for one caller | inline |
| A5 | Two benchmark numbers that read as one | edit the docs |
| B1 | The per-version table does not need an `Arc` | small, contained |
| B2 | `Server::run` is the accept loop and the connection body in one function | extract a function |
| B3 | `MakeDispatcher for Arc<Router<S>>` is implemented on the wrong side of the seam | move one impl |
| B4 | `Finished` carries the result twice | narrow the public surface |
| C1 | A refused peer never reaches `on_finish` | needs a decision on shape |
| C2 | `Listener::prepare` cannot see the listener, so no preamble can be configured | one associated type |
| C3 | `elapsed` excludes queueing delay but is documented as including it | two lines |
| C4 | Two tasks per connection, paid even when nothing reports | measure first |

---

## A. Duplicated and dead

### A1. `breakpoints()` runs twice for every build

`src/router.rs:251-252`:

```rust
let mut tables = Vec::with_capacity(breakpoints(&entries).len());
for version in breakpoints(&entries) {
```

The function allocates a `Vec`, walks every entry's ID table, sorts and dedups — and the first call
exists only to read `.len()` for a capacity hint. Bind it once. It is a startup cost and nobody will
ever notice it, but it is the only place in the crate where a pure function's result is thrown away
and immediately recomputed, and it reads like an oversight because it is one.

### A2. `RouterDispatcher::router()` is dead

`src/router.rs:452-455`. No callers in `src/` or `tests/`. This is the same class as `Router::inbound`
and `Router::unknown_policy`, removed last round for the same reason: a public accessor with no
reader is a promise made to nobody, and it pins a field's type into the API.

Delete it, or give it a use. (There is a plausible one — a `Dispatcher` wrapper that decorates a
`RouterDispatcher` would want to reach the router — but nothing in the crate does that today.)

### A3. Three handler traits, three blanket impls, one shape

`src/router.rs:75-120`. `Handler<S, P>`, `TickHandler<S>` and `ErrorHandler<S>` are the same
construction written three times: a trait with a single `call` method taking a `Ctx` and returning
`Result<()>`, plus a blanket impl over the corresponding `Fn`. Forty-six lines and three public
names, and the only difference between them is what sits after the `Ctx` argument.

Two of them can go with no change at the call site. `RouterBuilder` stores them as
`Arc<dyn TickHandler<S>>` / `Arc<dyn ErrorHandler<S>>`, and

```rust
tick: Option<Arc<dyn for<'c> Fn(Ctx<'c, S>) -> Result<()> + Send + Sync>>,
on_error: Option<Arc<dyn for<'c> Fn(Ctx<'c, S>, &Ending) -> Result<()> + Send + Sync>>,
```

is the same type with the trait spelled out — the module already writes exactly this shape for
`ErasedHandler` (`:122`). `.on_tick(|ctx| ...)` is unchanged.

`Handler<S, P>` can go too, and the call site gets *better*:

```rust
pub fn on<P: Packet>(
    mut self,
    handler: impl Fn(Ctx<'_, S>, P) -> Result<()> + Send + Sync + 'static,
) -> Self
```

turns `.on::<Intention, _>(on_intention)` into `.on::<Intention>(on_intention)`. The `_` that every
registration carries today exists only to stand for the handler type the trait bound introduced.

Both halves were checked by compiling a reduction rather than assumed: the elided `Ctx<'_, S>` does
become the `for<'c>` bound the erased box needs, and turbofishing `P` alone next to an
`impl Trait` argument is accepted.

**What is lost.** A caller can no longer implement the trait on a named struct — a stateful handler
must be a closure capturing an `Arc`, which is what the demo does anyway. That is a real if small
extensibility loss, so it is a decision rather than a cleanup. But as things stand, three
near-identical traits are being maintained for a capability nothing in the crate or its tests uses.

### A4. `Entry::id` wraps a free function for a single caller

`src/router.rs:133-136` is a three-line method around `crate::packet::ids(version, self.ids)`, called
once, from `build_table` (`:302`). Inline it and the `Entry` impl block disappears.

### A5. Two measurements that read as the same measurement

`src/conn/dispatch.rs:16-17` — "0.9 ns against a 12 ns table lookup". `src/router.rs:409-412` —
"dispatch two loads instead of a hash lookup and a pair of atomics: measured, 1.8 ns against 14 ns".

These are different comparisons (a virtual call against a static one; a cached table against a
per-frame lookup), but they sit close enough in both magnitude and phrasing that a reader takes them
for one number told twice, and neither says on what. Either give each a sentence saying what was
measured, or drop the digits and keep the shape of the claim. Numbers in doc comments outlive the
machine they were taken on.

---

## B. Simplifications worth making

### B1. The per-version dispatch table does not need to be behind an `Arc`

`src/router.rs:344`, `:394-403`, `:413-416`. Today: `tables: Box<[(ProtocolVersion, Arc<Table>)]>`,
and every accepted connection clones one of those `Arc`s into its `RouterDispatcher`.

But a `RouterDispatcher` already holds `Arc<Router<S>>`, and the tables live inside the router — so
the table it points at cannot outlive it, and the reference count is counting something that is
already guaranteed. Store the index:

```rust
pub struct RouterDispatcher<S> {
    router: Arc<Router<S>>,
    /// Index into `router.tables`, rebound by `set_version`.
    table: usize,
}
```

That removes an allocation per breakpoint, an atomic increment per accepted connection and a
decrement per finished one, and one wrapper type from the module. `Router::table` returns a `usize`;
`dispatch` reads `self.router.tables[self.table].1`. It is still *cached* — no search per frame,
which was the entire point of holding it — at the price of one more dereference and no atomics.
`Clone for RouterDispatcher` also becomes one `Arc` clone instead of two.

### B2. `Server::run` is two functions wearing one name

`src/server.rs:334-454`. About a hundred and twenty lines covering four concerns: taking an admission permit,
accepting and classifying accept errors, running one connection end to end, and draining.

The third — everything inside `tasks.spawn(async move { … })` (`:390-434`) — is the part a reader
wants to read on its own, and it is the part that will grow: C1 below adds reporting to it, and any
observability work lands there too. Extracted into a free `async fn` taking the handful of values it
captures, the accept loop fits on a screen and the connection body becomes testable without a
listener at all.

Incidentally, the two consecutive `select!`s both carry `() = self.shutdown.cancelled() => break`
(`:349`, `:358`). That is correct — they guard different awaits — but it is worth noticing that the
permit arm exists mostly to make the shutdown check happen twice, which is the sort of thing that
looks like a copy-paste and gets "cleaned up" by someone in a hurry. A comment would earn its keep.

### B3. `MakeDispatcher for Arc<Router<S>>` is implemented on the wrong side of the seam

`src/server.rs:127-146`. The crate has a stated pattern for seams, and `conn/dispatch.rs:1-6` states
it: *the consumer declares the trait, the provider implements it for its own type.* `conn` declares
`Dispatcher`; `router` implements it for `RouterDispatcher`; no type in `conn` names the router.

`MakeDispatcher` does it the other way round — `server` declares it **and** implements it for the
router's type — which is why `server.rs` imports `Router` and `RouterDispatcher` at all (`:49`).
Moving that one impl into `router.rs` removes `server`'s only dependency on `router`, and makes the
two seams read the same way. Nothing else changes; the trait is public either way.

### B4. `Finished` carries the result twice

`src/server.rs:176-200`. `Finished::result` and `Finished::outcome.unwrap().result` are the same
value whenever the connection did not panic, and nothing in the type says which one to read.

The asymmetry shows in the accessors: `state()` exists because reaching through `outcome` is
awkward, but `version` and `phase` got none — so a caller who wants those goes through `outcome`
anyway and ends up holding a second `result`.

`result` has to stay, because the panic case has no `Outcome` to take one from. So make `outcome`
private and finish the accessor set:

```rust
pub fn state(&self) -> Option<&S>
pub fn version(&self) -> Option<ProtocolVersion>
pub fn phase(&self) -> Option<Phase>
```

Each `Option` means the same thing — "unless it panicked" — and the public surface has exactly one
`result` on it. `demo::server::log_completion` already only uses `result`, `addr`, `elapsed` and
`state()`, so nothing in the crate needs the field.

---

## C. Behaviour gaps found while reading

### C1. A refused peer never reaches `on_finish`

`src/server.rs:394-407`. A `Listener::prepare` that fails and an `on_accept` that returns `false`
both `return` before anything is reported. So the hook whose whole purpose is to be the one place a
connection's fate is observed cannot see two of the fates:

* the preamble failed — a PROXY header that would not parse, a TLS handshake that was refused;
* we turned the peer away at the door.

Both are a `debug!` line and nothing else. Concretely: a per-IP rate limiter installed via
`on_accept` cannot be measured from `on_finish`, so the metric that says "how many did we refuse"
has nowhere to come from. This is the same class of hole as a panicking connection vanishing from
the report, which was worth closing.

It is not free to fix, because `Finished::result` is a `&Result<(), Ending>` and neither of these
has an `Ending` — they never reached the protocol, so nothing *ended*. Two honest options:

1. **A second hook**, `on_refused(&A, Refusal)`. Cheap, and splits reporting across two callbacks.
2. **Widen `Finished`** so "did not reach the protocol" is a case it can carry, and keep one hook.

The second is better and it is the one to argue for: one hook means one place metrics are emitted
and one match to keep exhaustive, which is the property that made folding the panic case into
`Ending::Failed` worth doing. The shape needs deciding — it should not be squeezed into `Ending`,
whose documented meaning is "what ended a connection that we did not end ourselves", and a refusal is
the opposite of that.

### C2. `Listener::prepare` cannot see the listener, so no preamble can be configured

`src/server.rs:97-102`. `prepare` is an associated function with no `&self`, and deliberately so: it
runs on the connection's task, so it cannot borrow the listener across tasks.

The consequence is that **a preamble needing any configuration cannot be written**:

* a TLS listener needs its `TlsAcceptor` — inherently per-listener state;
* a PROXY-protocol listener needs the set of proxies it trusts;
* a socket option needs a value to set.

This has bitten once already: `TCP_NODELAY` could not be made a setting last round for exactly this
reason and went in hardcoded, which is documented in `REVIEW_DECISIONS.md` as a deviation.

The fix is to let `accept` — which *does* see `&self` — hand the connection task whatever `prepare`
will need:

```rust
pub trait Listener: Send + 'static {
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;
    type Addr: fmt::Debug + Send + 'static;

    /// What `accept` produced, before the preamble has run.
    type Pending: Send + 'static;

    fn accept(&mut self) -> impl Future<Output = io::Result<Self::Pending>> + Send;

    fn prepare(
        pending: Self::Pending,
    ) -> impl Future<Output = io::Result<(Self::Io, Self::Addr)>> + Send;
}
```

`Pending` is `(TcpStream, SocketAddr)` for the plain case — the default `prepare` stays a no-op — and
`(TcpStream, SocketAddr, Arc<TlsAcceptor>)` for TLS, where the `Arc` clone is taken in `accept`, on
the listener's task, where `&self` is available. The accept loop still awaits nothing but `accept`,
which was the property `prepare` was introduced to protect.

**Cost:** one associated type, and implementors of a configured listener write a small struct. That
is the price of the hook being usable for the two things it exists for.

### C3. `elapsed` excludes queueing delay but says it does not

`src/server.rs:177-178` documents `Finished::elapsed` as "from the accept to the last byte".
`src/server.rs:391` takes the instant on the *first line inside the spawned task*.

Under load — precisely when the number is worth having — the gap between `tasks.spawn` and the first
poll of that task is real queueing delay, and it is silently excluded. So the metric flatters the
server exactly when the server is struggling.

Take the `Instant` in the loop before `tasks.spawn` and move it into the task. Two lines, and the
doc comment becomes true.

### C4. Two tasks per connection, paid even when nothing will read the result

`src/server.rs:390` and `:415`. The outer task is what `TaskTracker` tracks for draining; the inner
`tokio::spawn(connection.run())` exists so a panic in a handler is observable rather than unwinding
past the report. That reasoning is sound and documented.

But it is a second `tokio::spawn` for every accepted connection, and it is paid unconditionally —
including when `on_finish` is `None` (`:417`), where the outcome is awaited and then dropped.

`futures::FutureExt::catch_unwind` over `AssertUnwindSafe(connection.run())` does the same job on one
task. The caveat is real and should be stated rather than waved past: `AssertUnwindSafe` is a claim
about what happens after a panic, and the claim here is defensible — the only thing observed
afterwards is *that* it panicked, and `S` is dropped rather than read. Under `panic = "abort"`
neither approach does anything.

Worth measuring before changing. Worth writing down either way that the second spawn is a choice with
a price, because right now the comment explains why it is correct without noting what it costs.

---

## D. Construction: four things built four ways

The crate builds four things and uses a different idiom for each:

| What | How |
|---|---|
| `Router` | `Router::builder()` … `.build() -> Result<Router, BuildError>` — fallible builder, separate types |
| `Server` | `serve(listener, make, state)` … `.await` — free function; builder and value are one type; infallible |
| `Connection` | `Connection::new(io, dispatcher, state, config, shutdown) -> (Connection, Handle)` — five positional arguments, returns a tuple |
| `ConnectionConfig` | `ConnectionConfig { tick_interval: …, ..Default::default() }` — public fields |

The crate-level example uses three of them in nine lines (`src/lib.rs:80-103`), which is where it
shows: `Arc::new(router()?)`, then a struct literal with `..Default::default()`, then a free function
with three positional arguments and chained setters. Each is defensible alone; together they mean
there is no answer to "how do you make one of these in this crate".

Given the stated preference, the consistent answer is `X::builder()`.

### D1. `Server::builder()`

```rust
Server::builder()
    .listener(listener)
    .dispatch(router)                    // Arc<Router<S>>, or make_with(..)
    .state(|addr| Session { peer: Some(*addr), ..Session::default() })
    .max_lifetime(Duration::from_secs(60))
    .max_connections(10_000)
    .graceful_shutdown(shutdown)
    .on_finish(log_completion)
    .serve()
    .await;
```

Typestate carries the three required arguments: `ServerBuilder<L, M, F>` starts as
`ServerBuilder<(), (), ()>` and each of the three setters swaps one parameter, so `.serve()` only
exists once all three are set — "you cannot forget one" survives the move away from positional
arguments, which is the thing a plain builder usually gives up.

This also makes §E1 free: the builder is where the callback type parameters get threaded, and the
builder's own type is never written down by anyone.

Keep `serve(listener, make, state)` as the two-line shorthand if it earns its keep; it is the shape
the README leads with, and there is no reason a crate cannot have both as long as one is defined in
terms of the other.

### D2. Forward the connection knobs, or give `ConnectionConfig` a builder

`.config(ConnectionConfig)` (`src/server.rs:260`) is where the builder stops and struct-literal-with-
`..default()` starts. Either forward the five knobs onto the server builder (`.limits()`,
`.tick_interval()`, `.max_lifetime()`, `.close_timeout()`, `.initial_phase()`) or give
`ConnectionConfig` a builder of its own and keep `.config()` for the wholesale case.

Forwarding is fewer concepts and reads better at the call site. A `ConnectionConfig::builder()` is
the better answer if `Connection::builder()` happens too, since then both paths share it.

### D3. `Connection::builder()`

`Connection::new(io, dispatcher, state, config, shutdown)` is five positional arguments returning a
tuple, and it is the constructor most likely to be called wrong — `dispatcher`, `state` and `config`
are all "some `S`-shaped thing" at a glance. It is also the one the tests call most, and every test
helper in `tests/` wraps it to avoid repeating the argument list.

Lower priority than D1, because it is not on the common path — but it is the place where positional
arguments actually cost something today.

### D4. One setter is named unlike the others

`with_graceful_shutdown` (`src/server.rs:271`) is the only `with_`-prefixed setter; the rest are bare
nouns (`config`, `drain_timeout`, `max_connections`) and `on_`-verbs (`on_accept`, `on_finish`). The
name is borrowed from hyper and axum, so there is a case for keeping it on familiarity grounds. There
is no case for it being the only one of its kind.

---

## E. Type parameters and boxes

The request is that types propagate their parameters instead of erasing to `Box`/`Arc<dyn>`. That is
right in some of these places and wrong in others, and the line between them is sharp enough to write
down:

> **Erase what has to be named. Propagate what is only ever built and consumed in one expression.**

A type parameter is free when nobody writes the type down. It becomes expensive the moment someone
has to name it — in a struct field, a function's return type, a `static` — because at that point the
parameter is not an implementation detail of ours, it is in *their* signature too, and it propagates
outward until it reaches something that has to erase it anyway.

Applied here:

| Where | Today | Verdict |
|---|---|---|
| `Server::on_accept`, `on_finish` (`server.rs:203-206`) | `Option<Arc<dyn Fn …>>` | **propagate** — E1 |
| `Server::state` (`:215`) | `Arc<F>` — already a parameter | the precedent, in the same struct |
| `Server::IntoFuture` (`:465`) | `BoxFuture<'static, ()>` | **blocked by the language** — E3 |
| `Router::tick`, `on_error` (`router.rs:345-346`) | `Arc<dyn …>` | **keep erased** — E2 |
| `Router` packet handlers (`:122`) | `Box<dyn Fn …>` per packet | **keep erased** — heterogeneous by construction; this is the design |
| `Op::With`, `Op::Spawn`, `Op::Encrypt` | `Box<dyn …>` | **keep erased** — one queue holds all of them; F5 in the previous round |
| `RouterDispatcher::table` (`:415`) | `Arc<Table>` | **remove the indirection entirely** — B1 |

### E1. `Server`'s two callbacks should be type parameters

`Server` is built and awaited in a single expression; nobody stores one or names its type. `state` is
already `Arc<F>` in the very same struct, so the pattern is there and the two callbacks are the
inconsistency, not the other way round.

For "not set", a unit type implementing the trait as a no-op beats `Option<G>`: it removes the branch
per connection as well as the allocation.

```rust
pub trait Admit<A> { fn admit(&self, addr: &A) -> bool; }
impl<A> Admit<A> for () { fn admit(&self, _: &A) -> bool { true } }

pub trait Report<S, A> { fn report(&self, finished: &Finished<'_, S, A>); }
impl<S, A> Report<S, A> for () { fn report(&self, _: &Finished<'_, S, A>) {} }
```

Two `Arc` allocations and two virtual calls go away. Be honest about the size of that: the virtual
call happens once per *finished connection*, so the argument here is consistency with `state` and
with `MakeDispatcher`, not throughput.

**One wrinkle worth knowing before starting.** `S` is currently used by the struct only through
`on_finish: Option<OnFinish<S, L::Addr>>` (`:221`). Make that a parameter and `S` is unconstrained,
which Rust rejects. Two ways out, and the second is better:

* add `PhantomData<fn() -> S>` — the usual price;
* **drop `S` from the struct entirely.** It is recoverable on the impl block from `F`'s output —
  `impl<L, S, F, M, A, R> Server<L, F, M, A, R> where F: Fn(&L::Addr) -> S` constrains `S` through
  the `Fn` bound's associated type. Checked by compiling a reduction, because the rule that governs
  it (an impl parameter must be constrained by the self type, the trait ref *or a predicate*) is
  exactly the kind that reads as if it might not apply. `Server` loses a parameter instead of
  gaining a marker field.

### E2. `Router`'s handlers must stay erased — and this is the case that proves the rule

Making them parameters gives `Router<S, T, E>`. Unlike `Server`, `Router` is named constantly: it
lives in an `Arc` for the life of the process, it is a field in an application struct, and it is the
return type of a factory function. The demo's is:

```rust
pub fn router() -> std::result::Result<Router<Session>, BuildError>
```

With the handlers as parameters, and the handlers being closures or `fn` items, that signature has
nothing to write — a closure type has no name. It becomes

```rust
-> Result<Router<Session, impl TickHandler<Session>, impl ErrorHandler<Session>>, BuildError>
```

which is legal, and then cannot be stored in a struct field without adding two more parameters to
*that* struct, which cannot be stored without adding two more to the next one out. The erasure has to
happen somewhere; the only question is whether it happens once, here, or is pushed into every
application that uses the crate.

This is the same reason `Box<dyn Error>` exists. It is not laziness — it is the boundary where the
parameter stops being ours. What it costs is two `Arc<dyn>` and one virtual call per tick, a tick
being a once-per-sixteen-seconds event.

The same argument covers the packet handlers (`:122`) even more strongly: they are heterogeneous by
construction — one table, many packet types — so there is no parameter to propagate in the first
place. That erasure is the module's central design decision and its docs already say so.

### E3. `IntoFuture` is boxed because the language has not shipped the alternative

`type IntoFuture = impl Future<Output = ()>` requires TAIT, which is unstable, and the state machine
of an `async fn` body has no nameable type — so there is nothing else to put in that associated type
position. `Server::run` is already public and unboxed and the doc already points at it
(`src/server.rs:211`).

Nothing to do. Recorded so it is not re-litigated: this box is not a design choice.

---

## F. Smaller things

* **`Server` is not `#[must_use]`.** `serve` is (`:230`), so `serve(..)` without `.await` warns — but
  a `Server` produced any other way (a builder, per D1) and dropped does nothing, silently.
* **`ConnectionConfig` could be `Copy`.** It is cloned per connection (`src/server.rs:384`); every
  field is `Copy` already, so the `Clone` is a memcpy that reads like an allocation.
* **`MAX_PACKETS` is checked after the fact.** `build()` rejects an over-large registration
  (`router.rs:243-248`), but `on()` pushes unbounded, so the memory is allocated first and the error
  arrives second. Theoretical at `u16::MAX` packets.
* **`RouterBuilder::on` returns early on a *previous* packet's error** (`:191-193`), so later
  collision diagnostics are computed against an incomplete entry set. Harmless — `build` reports the
  first error regardless — but the early return reads as though it were about the packet being
  registered.
* **`Table::by_phase` is five boxed slices** (`:145`). One flat slice with five offsets is one
  allocation per breakpoint instead of five. Not worth doing at five phases and a handful of tables
  unless startup allocation counts start to matter.
* **`is_peer_error`** (`:487`): the `ConnectionRefused` arm flagged in the previous review is gone.
  No finding — recorded so it is not reported a third time.

---

## G. What should not be traded away while fixing the above

Load-bearing, and easy to lose in a refactor:

* **The `Dispatcher` seam.** `conn` depends on the trait, not on `Router`. It is what lets a
  connection be driven by a test double, and `tests/dispatch.rs` exists to fail if the dependency
  creeps back.
* **Tables built once, at startup.** An ID collision is a boot failure, not a runtime error on the
  first client that happens to send the packet. Any change to `build()` has to keep that.
* **`MakeWith`.** It looks like a wrapper that could be replaced with a blanket
  `impl<F: Fn() -> D> MakeDispatcher for F`, and it cannot: that overlaps the `Arc<Router<S>>` impl
  as far as coherence is concerned. The comment says so; keep it.
* **The permit is taken before the accept** (`:345-352`). A full server leaves the next connection in
  the kernel's backlog instead of accepting it in order to drop it — backpressure rather than a
  refusal the peer cannot distinguish from an outage.
* **`CancelOnDrop`** (`:475-481`). Dropping the server future — losing a `select!` against a signal —
  would otherwise orphan every live connection with nothing able to stop them.
* **Accept errors are never fatal** (`:363-377`). The failure that happens in practice is running out
  of file descriptors, which is transient; a server that exits on it turns a busy minute into an
  outage.
