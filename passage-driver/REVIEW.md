# Review: `router.rs` and `server.rs`

A close read of the two modules that assemble everything else, looking for code that was duplicated,
dead, or more complicated than the job needed. Two directions were folded in as sections of their
own: **construction should use builders** (§D) and **types should carry their parameters rather than
erasing to boxes** (§E). The second one turned out to be right in some places and wrong in others,
and the line between them is the most useful thing in this document.

**Every finding below has been applied**, except those marked *Not done* or *withdrawn*, with the
reasons given in place. C1 and C2 are the interesting ones: both were built, both were then taken
back out, and §F says what replaced them. Sections are kept in their original order so the argument for each change is still
readable next to what it became; line numbers are current as of this commit.

At a glance:

| | Finding | What it became |
|---|---|---|
| A1 | `breakpoints()` was computed twice per build | bound once |
| A2 | `RouterDispatcher::router()` had no callers | deleted |
| A3 | Three handler traits were the same construction three times | three type aliases; `.on::<P>(f)` |
| A4 | `Entry::id` wrapped a free function for one caller | inlined |
| A5 | Two benchmark numbers that read as one | both replaced with what they were claiming |
| B1 | The per-version table did not need an `Arc` | an index into `router.tables` |
| B2 | `Server::run` was the accept loop and the connection body | `connection()`, a free `async fn` |
| B3 | `MakeDispatcher for Arc<Router<S>>` was on the wrong side of the seam | moved to `router.rs` |
| B4 | `Finished` carried the result twice | **superseded** -- there is no report to carry it |
| C1 | A refused peer never reached `on_finish` | **withdrawn** -- each layer counts its own |
| C2 | `Listener::prepare` could not be configured | right finding, wrong fix -- it is a `Layer` |
| C3 | `elapsed` excluded queueing delay | the clock starts in the accept loop |
| C4 | Two tasks per connection | one, with `catch_unwind` |
| D | Four things built four ways | `X::builder()` for all three, typestate on the server |
| E | Boxes that should be parameters, and boxes that should not | `Server` propagates; `Router` erases |
| F1 | Two bespoke mechanisms for one job, and a central reporter | one `Layer` stack; the driver logs its own |

86 tests pass, plus doctests; clippy and rustfmt are clean and `cargo doc` emits no warnings.

---

## A. Duplicated and dead

### A1. `breakpoints()` ran twice for every build

```rust
let mut tables = Vec::with_capacity(breakpoints(&entries).len());
for version in breakpoints(&entries) {
```

The function allocates a `Vec`, walks every entry's ID table, sorts and dedups -- and the first call
existed only to read `.len()` for a capacity hint. It was a startup cost nobody would ever notice,
but it was the only place in the crate where a pure function's result was thrown away and
immediately recomputed, and it read like an oversight because it was one.

**Resolved.** Bound once (`router.rs:223-227`).

### A2. `RouterDispatcher::router()` was dead

No callers in `src/` or `tests/`. The same class as `Router::inbound` and `Router::unknown_policy`,
removed a round earlier for the same reason: a public accessor with no reader is a promise made to
nobody, and it pins a field's type into the API.

**Resolved.** Deleted.

### A3. Three handler traits, three blanket impls, one shape

`Handler<S, P>`, `TickHandler<S>` and `ErrorHandler<S>` were the same construction written three
times: a trait with a single `call` method taking a `Ctx` and returning `Result<()>`, plus a blanket
impl over the corresponding `Fn`. Forty-six lines and three public names, and the only difference
between them was what sat after the `Ctx` argument.

**Resolved.** All three are now type aliases for the function types they always described
(`router.rs:70-90`), which is the shape the module already used for `ErasedHandler`:

```rust
type TickHandler<S> = Arc<dyn for<'c> Fn(Ctx<'c, S>) -> Result<()> + Send + Sync>;
```

`on` takes `impl Fn(Ctx<'_, S>, P) -> Result<()> + Send + Sync + 'static`, so every registration in
the crate lost a `_`: `.on::<Intention, _>(on_intention)` is now `.on::<Intention>(on_intention)`.
The elided `Ctx<'_, S>` does become the `for<'c>` bound the erased box needs, and turbofishing `P`
alone beside an `impl Trait` argument is accepted -- both checked by compiling a reduction rather
than assumed.

**What was given up**, as predicted: a caller can no longer implement the trait on a named struct, so
a stateful handler must be a closure capturing an `Arc`. That is what the demo and every test did
anyway, and three near-identical traits were being maintained for a capability nothing used.

### A4. `Entry::id` wrapped a free function for a single caller

Three lines around `crate::packet::ids(version, self.ids)`, called once.

**Resolved.** Inlined; the `Entry` impl block is gone.

### A5. Two measurements that read as the same measurement

`dispatch.rs` claimed "0.9 ns against a 12 ns table lookup"; `router.rs` claimed "two loads instead
of a hash lookup and a pair of atomics: measured, 1.8 ns against 14 ns". Different comparisons, close
enough in magnitude and phrasing to be taken for one number told twice, and neither said on what.

**Resolved.** Both now state the shape of the claim without the digits -- the virtual call is "the
cheapest thing on the path", and the cached table "turns dispatch into a pair of indexed loads,
where looking the version up per frame would repeat a binary search over the breakpoints for every
packet". Numbers in doc comments outlive the machine they were taken on.

---

## B. Simplifications

### B1. The per-version dispatch table did not need to be behind an `Arc`

`tables: Box<[(ProtocolVersion, Arc<Table>)]>`, and every accepted connection cloned one of those
`Arc`s into its `RouterDispatcher` -- which already held `Arc<Router<S>>`, inside which the tables
live. The table it pointed at could not outlive the router, so the reference count was counting
something already guaranteed.

**Resolved.** `Router::table` returns a `usize` (`router.rs:371-377`) and `RouterDispatcher` stores
it. That removes an allocation per breakpoint, an atomic increment per accepted connection and a
decrement per finished one, and a wrapper type from the module. It is still *resolved once* rather
than per frame, which was the entire point of caching it, and `Clone for RouterDispatcher` became
one `Arc` clone instead of two.

### B2. `Server::run` was two functions wearing one name

About a hundred and twenty lines covering four concerns: taking an admission permit, accepting and
classifying accept errors, running one connection end to end, and draining.

**Resolved.** The third is now `connection()`, a free `async fn` (`server.rs:733-806`). The accept
loop fits on a screen, and the connection body -- where C4 lands, and where anything else about a
running connection would -- reads without a listener anywhere in sight.

The two consecutive `select!`s both carry `() = self.shutdown.cancelled() => break`, which is
correct (they guard different awaits) but reads like a copy-paste waiting to be "cleaned up". It now
carries a comment saying what the second one is for.

### B3. `MakeDispatcher for Arc<Router<S>>` was implemented on the wrong side of the seam

The crate has a stated pattern for seams, and `conn/dispatch.rs` states it: *the consumer declares
the trait, the provider implements it for its own type.* `conn` declares `Dispatcher`; `router`
implements it for `RouterDispatcher`; no type in `conn` names the router. `MakeDispatcher` did it the
other way round -- `server` declared it **and** implemented it for the router's type -- which was the
only reason `server.rs` imported `Router` at all.

**Resolved.** The impl moved to `router.rs:435-441`. `server` no longer names a router anywhere, and
the two seams read the same way.

### B4. `Finished` carried the result twice -- **superseded by F1**

`Finished::result` and `Finished::outcome.unwrap().result` were the same value whenever the
connection did not panic, and nothing in the type said which to read. The asymmetry showed in the
accessors: `state()` existed because reaching through `outcome` was awkward, but `version` and
`phase` got none.

It was fixed as proposed -- `outcome` private, the accessor set finished -- and then the whole type
went with `on_finish`; see F1. Worth keeping the note, because the symptom was real and was pointing
at something bigger than itself: a struct that cannot say which of its two fields to read is usually
a struct that is doing a job nobody asked it to do.

---

## C. Behaviour gaps found while reading

### C1. A refused peer never reaches `on_finish` -- **withdrawn, and the finding was wrong**

The claim was that a `Listener::prepare` that failed and an `on_accept` that returned `false` both
`return` before anything is reported, so the hook whose purpose is to be the one place a
connection's fate is observed cannot see two of the fates -- and concretely, that a per-IP rate
limiter installed via `on_accept` could not be measured.

That last step does not follow, and it is where the finding went wrong. **A rate limiter that
refuses a peer is the thing that knows it refused it.** It is holding the bucket that was empty and
the address that emptied it; incrementing a counter there is one line, at the point where the reason
is still in hand. Nothing has to be passed anywhere.

This was built as proposed -- a `Refusal` enum, a `Conclusion` enum, `Finished` widened to carry
either -- and then reverted, because the review had argued itself into the position this crate
already rejects one level down. From the README, written before any of this:

> Note what the driver is *not* asked to remember. There is one `Completion::Closed`, not a second
> variant for "we refused them" -- the handler that refused knows it did, and writes the reason into
> its own state.

The admission check is that same argument one layer up. What the widening actually bought was: two
public enums, a `Finished` whose every accessor became an `Option` for two unrelated reasons, a
reporting hook that had to grow an arm for each layer upstream of it, and -- the real cost -- a rule
that each layer must flatten what it knows into a vocabulary `server.rs` invents for it.
`Refusal::Preamble(io::Error)` is a fair summary of nothing: the listener that produced it knew
whether it was a truncated PROXY header or a rejected certificate, and threw that away to fit.

**What is there instead.** `Admit` takes `&self`, so an implementation can be a named type holding a
counter, a bucket and a clock; its documentation says that counting refusals is its job and shows
the three lines. `tests/server.rs` has a `RateLimiter` that does exactly that, and asserts both that
its own count is right *and* that `on_finish` saw only the connection that ran. `Finished` is back to
`result` plus the three `Outcome` accessors -- B4 as originally proposed, and nothing more.

The one thing genuinely worth keeping from the exercise is in §C2: a preamble that fails should not
lose the address it failed on, which is why `accept` returns it.

### C2. `Listener::prepare` could not see the listener, so no preamble could be configured

`prepare` was an associated function with no `&self`, deliberately: it runs on the connection's task,
so it cannot borrow the listener across tasks. The consequence was that **a preamble needing any
configuration could not be written** -- a TLS listener needs its `TlsAcceptor`, a PROXY listener
needs the set of proxies it trusts, a socket option needs a value. This had already bitten once:
`TCP_NODELAY` could not be made a setting and went in hardcoded.

**The finding was right; the fix was wrong.** It was fixed with a `Pending` associated type, so that
`accept` -- which *does* see `&self` -- could hand the connection task whatever `prepare` would
need. That works, and it is a workaround for a misplacement: reading a PROXY header is not something
a *listener* does. It is something done *to an accepted socket*, which is equally true of a TLS
handshake and of a rate limiter, and those three had two separate mechanisms and two special cases in
the connection body between them.

Replaced by the [`Layer`] stack -- see F1. `Listener` is now a source of sockets and nothing else:
two associated types, one method, no default to override.
### C3. `elapsed` excluded queueing delay but said it did not

`Finished::elapsed` was documented as "from the accept to the last byte" while the `Instant` was
taken on the first line inside the spawned task. Under load -- precisely when the number is worth
having -- the gap between `spawn` and the first poll of that task is real queueing delay, and it was
silently excluded, so the metric flattered the server exactly when the server was struggling.

**Resolved.** The clock starts in the accept loop (`server.rs:666-668`) and is passed in.

### C4. Two tasks per connection, paid even when nothing would read the result

The outer task is what `TaskTracker` tracks for draining; the inner `tokio::spawn(connection.run())`
existed so a panic in a handler was observable rather than unwinding past the report. Sound, and
documented -- but a second `tokio::spawn` for every accepted connection, paid unconditionally.

**Resolved.** `AssertUnwindSafe(connection.run()).catch_unwind()` does the same job on one task
(`server.rs:781-805`). The caveat is stated rather than waved past: `AssertUnwindSafe` is a claim
about what is observed after a panic, and here the only thing observed is *that* it panicked -- the
state is dropped, never read. Under `panic = "abort"` neither approach does anything. The panic
payload is turned into a message by `panic_message`, so the existing test still sees
`Class::Internal` and the label `panic`.

---

## D. Construction: four things built four ways

The crate built four things with a different idiom for each -- a fallible builder, a free function
whose builder and value were one type, a five-argument constructor returning a tuple, and a
public-fields struct. The crate-level example used three of them in nine lines, which is where it
showed: there was no answer to "how do you make one of these in this crate".

There is now one answer, `X::builder()`, and the nine-line example is one chain.

### D1. `Server::builder()`

**Resolved.** `Server` *is* the builder: `Server<L, F, M, A, R>` starts as `Server<(), (), ()>` and
each of the three required setters replaces one parameter, so `run()` -- and `IntoFuture`, and
therefore `.await` -- exist only once a listener, something to dispatch to and a state factory have
all been given. "You cannot forget one" survives the move away from positional arguments, which is
the thing a plain builder usually gives up.

The setters are split across four impl blocks by what each needs to know, and that is not
decoration: a setter that takes a closure has to be able to *type* it, and `|addr| ...` can only be
inferred from a bound that names `Fn` directly. So `.state()` and `.on_accept()` carry their bounds
and the chain is written listener-first.

`serve(listener, make, state)` is kept as the positional shorthand, defined as those three setters.

### D2. The connection knobs are forwarded

`.config(ConnectionConfig)` was where the builder stopped and struct-literal-with-`..default()`
started. The five knobs -- `limits`, `tick_interval`, `max_lifetime`, `close_timeout`,
`initial_phase`, and `initial_version` for completeness -- are now setters on the server, with
`.config()` kept for the wholesale case.

Forwarding was chosen over a `ConnectionConfig::builder()`: `ConnectionConfig` is plain data with
public fields and a `Default`, and a builder for it would have been a fourth idiom for a struct
literal. It is now also `Copy` (see §F).

### D3. `Connection::builder()`

`Connection::new(io, dispatcher, state, config, shutdown)` was five positional arguments returning a
tuple, and the constructor most likely to be called wrong -- `dispatcher`, `state` and `config` all
look like "some `S`-shaped thing" at a glance. It was also the one the tests called most.

**Resolved.** `Connection::builder(io, dispatcher, state)` with `.config()`, `.shutdown()` and
`.build()`. The three arguments that stayed positional are the ones a connection cannot exist
without *and* are of three unmistakably different kinds; the two that were confusable are now set by
name. `Connection::new` is private.

Typestate was not used for `state`, unlike the server: `()` is a perfectly legitimate `S` -- one test
uses it -- so it cannot double as the marker for "unset".

### D4. One setter was named unlike the others

`with_graceful_shutdown` was the only `with_`-prefixed setter; the rest were bare nouns and
`on_`-verbs. **Resolved:** `graceful_shutdown`.

---

## E. Type parameters and boxes

The request was that types propagate their parameters instead of erasing to `Box`/`Arc<dyn>`. That is
right in some places and wrong in others, and the line between them is sharp enough to write down:

> **Erase what has to be named. Propagate what is only ever built and consumed in one expression.**

A type parameter is free when nobody writes the type down. It becomes expensive the moment someone
has to name it -- in a struct field, a function's return type, a `static` -- because at that point the
parameter is not an implementation detail of ours, it is in *their* signature too, and it propagates
outward until it reaches something that has to erase it anyway.

| Where | Was | Verdict |
|---|---|---|
| `Server::on_accept`, `on_finish` | `Option<Arc<dyn Fn …>>` | **propagated** -- E1 |
| `Server::state` | `Arc<F>` -- already a parameter | the precedent, in the same struct |
| `Server::IntoFuture` | `BoxFuture<'static, ()>` | **blocked by the language** -- E3 |
| `Router::tick`, `on_error` | `Arc<dyn …>` | **kept erased** -- E2 |
| `Router` packet handlers | `Box<dyn Fn …>` per packet | **kept erased** -- heterogeneous by construction |
| `Op::With`, `Op::Spawn`, `Op::Encrypt` | `Box<dyn …>` | **kept erased** -- one queue holds all of them |
| `RouterDispatcher::table` | `Arc<Table>` | **indirection removed entirely** -- B1 |

### E1. `Server`'s callbacks are type parameters, not boxes

A `Server` is built and awaited in a single expression; nobody stores one or names its type. `state`
was already `Arc<F>` in the very same struct, so the boxed callbacks beside it were the
inconsistency.

**Resolved**, and it outlived the callbacks themselves: `on_accept` became the layer stack and
`on_finish` went away entirely (F1), but both are still carried as `A` rather than boxed, and the
unset case is still a `()` that compiles away rather than an `Option` tested per connection. The
layer stack needs it for a second reason the predicate never did -- `Stack<A, B>` has to name
`A::Io` to let a layer change the socket type, which an `Arc<dyn>` cannot express at all.

**The wrinkle, and the way out.** `S` was used by the struct only through its callbacks; making those
parameters left `S` unconstrained, which Rust rejects. Rather than add `PhantomData<fn() -> S>`, `S`
was **dropped from the struct entirely** and recovered on the impl blocks from `F`'s output --
`where F: Fn(&L::Addr) -> S` constrains it through the `Fn` bound's associated type. Checked by
compiling a reduction, because the rule that governs it (an impl parameter must be constrained by the
self type, the trait ref *or a predicate*) is exactly the kind that reads as if it might not apply.
`Server` lost a parameter instead of gaining a marker field.

The one cost worth recording: a closure passed to a setter whose bound is a *trait* rather than `Fn`
needs its parameter annotated, because inference only flows from a bound naming `Fn` directly. That
is why `.state(|addr| ...)` infers and `.layer(|addr: &SocketAddr| ...)` does not.
### E2. `Router`'s handlers stay erased -- and this is the case that proves the rule

Making them parameters gives `Router<S, T, E>`. Unlike `Server`, `Router` is named constantly: it
lives in an `Arc` for the life of the process, it is a field in an application struct, and it is the
return type of a factory function. The demo's is `fn router() -> Result<Router<Session>, BuildError>`.

With the handlers as parameters, and the handlers being closures or `fn` items, that signature has
nothing to write -- a closure type has no name. It becomes
`-> Result<Router<Session, impl TickHandler<Session>, impl ErrorHandler<Session>>, BuildError>`,
which is legal, and then cannot be stored in a struct field without adding two more parameters to
*that* struct, which cannot be stored without adding two more to the next one out. The erasure has to
happen somewhere; the only question is whether it happens once, here, or is pushed into every
application that uses the crate.

This is the same reason `Box<dyn Error>` exists. It is not laziness -- it is the boundary where the
parameter stops being ours. What it costs is two `Arc<dyn>` and one virtual call per tick, a tick
being a once-per-sixteen-seconds event.

The same argument covers the packet handlers even more strongly: they are heterogeneous by
construction -- one table, many packet types -- so there is no parameter to propagate in the first
place.

### E3. `IntoFuture` is boxed because the language has not shipped the alternative

`type IntoFuture = impl Future<Output = ()>` requires TAIT, which is unstable, and the state machine
of an `async fn` body has no nameable type. `Server::run` is the same future, public and unboxed, for
anyone who minds. Recorded, at the box itself, so it is not re-litigated: this one is not a design
choice.

---

## F. One mechanism instead of three

### F1. Two bespoke mechanisms for one job, and a reporter told about all of it

Three things could happen to a peer between the accept and the protocol, and they had two separate
mechanisms and two special cases in the connection body:

* a preamble, through [`Listener::prepare`] and the `Pending` associated type it needed (C2);
* an admission check, through `Server::on_accept`;
* and both of them reporting to `Server::on_finish`, which was also the reporter for everything a
  connection did (C1).

They are one shape. A PROXY header rewrites the address; a TLS handshake replaces the socket; a rate
limiter refuses. All three take a socket and an address and hand back a socket and an address, or
refuse -- so there is one trait for them, and the accept loop knows about none of them individually:

```rust
pub trait Layer<Io, Addr>: Send + Sync + 'static {
    type Io: AsyncRead + AsyncWrite + Send + Unpin + 'static;
    fn admit(&self, io: Io, addr: Addr)
        -> impl Future<Output = Option<(Self::Io, Addr)>> + Send;
}
```

`Option`, not `Result`: **`None` carries no reason because the layer that refused is the one holding
it** -- the expired bucket, the untrusted source, the certificate that failed to verify. A `Result`
would only let a layer summarise what it already knows into a vocabulary `server.rs` had to invent,
and hand it somewhere further away. This is C1's conclusion, now enforced by the signature.

Three stock impls carry everything: `()` (no layers, the identity), any `Fn(&Addr) -> bool` (the
predicate case), and `Stack<A, B>` (composition, which is how `Server::layer` accumulates them).
`Stack` is why `Io` is an associated type rather than fixed -- it names `A::Io` to let a layer change
the socket type, which a boxed callback could not have expressed.

**What went.** `Listener::Pending`, `Listener::prepare` and its `&mut Addr`, the identity `prepare`
every trivial listener had to write, `Server::on_accept`, and both special cases in the connection
body -- which is now one `let else`. `Listener` is two associated types and one method.

### F2. The reporting hook, and what replaced it

`on_finish` was the last place the crate handed everything to a central authority, and the objection
to it is the same one that killed C1, one level further out: a hook that has to be told every fate
makes each layer flatten what it knows, and puts the only complete picture in the place furthest
from where any of it happened.

What the driver could honestly report was never the application's facts -- it was its own. So it
reports those itself, and nothing else: an `info` span per connection carrying the peer, and one
event when a connection ends with the duration, the [`Ending`]'s label, and the version and phase it
reached -- at `warn` when the cause was ours, `debug` otherwise.

Everything a handler logs lands inside that span, which is what a central reporter was really
providing: correlation. The demo shows the shape -- `on_error` now logs the player and the reason
where it refuses them, instead of stashing a label in `Session::refused` for a reporter to read back
later. (The field stays: it is what `Outcome` hands to a caller driving a connection without the
accept loop, and `tests/ending.rs` asserts on it.)

`Finished`, `Report`, `Server::on_finish` and `demo::log_completion` are gone, and `Server` lost a
type parameter with them.

**The cost, stated plainly.** The driver's own reporting is now the only end-of-connection record, so
a caller wanting to aggregate endings their own way writes a `tracing` layer rather than a closure.
And it made the server tests read the log instead of a `Vec` -- which is arguably the better test,
since the log line is now the feature. `tests/common` grows a ~40-line recorder for it.

---

## G. Smaller things

* **`Server` was not `#[must_use]`.** Now it is, on the type, which also covers every setter --
  `Server::builder()…` dropped without awaiting warns. (The per-method `#[must_use]`s came off;
  clippy flags them as redundant once the type carries one.)
* **`ConnectionConfig` is now `Copy`.** Every field already was, so the per-connection `Clone` read
  like an allocation and was a memcpy.
* **`MAX_PACKETS` is now checked in `on()`** rather than after the fact in `build()`, so the memory
  is not allocated first and the error second. Theoretical at `u16::MAX` packets, but it is one
  comparison.
* **`RouterBuilder::on` no longer returns early** when a packet's ID table is out of order. It
  records the error and registers the entry anyway, so the entry set stays complete and a collision
  between two *other* packets is still found. `build` reports whichever mistake came first either
  way.
* **Not done: `Table::by_phase` is five boxed slices.** One flat slice with five offsets would be one
  allocation per breakpoint instead of five. Not worth doing at five phases and a handful of tables
  unless startup allocation counts start to matter, and it would trade a clear type for arithmetic.
* **`is_peer_error`:** the `ConnectionRefused` arm flagged in an earlier review is gone. No finding --
  recorded so it is not reported a third time.

---

## Where the implementation departs from what this review proposed

Five places, all where writing the code showed the proposal was not quite right:

1. **C1 was built and then removed**, and C2's fix with it. Both are recorded where they were
   proposed rather than deleted, because the reasoning that produced them is the reasoning worth not
   repeating: two findings about *things not being reported* were both really findings about the
   report existing at all.
2. **A `Layer` may not change the `Addr` type**, only its value. It could -- one more associated
   type -- and a TLS layer could then hand the state factory a verified client identity instead of a
   socket address. It is not worth it yet: `.state()` would bind to the stack's output rather than
   the listener's, so adding a layer later would silently change what the factory receives.
3. **D1 has no separate `ServerBuilder` type and no `.serve()`.** `Server` is the builder; a second
   type would have been two names for one thing, and `.serve()` a third name for `run()`/`.await`.
4. **D2 forwards the connection knobs instead of giving `ConnectionConfig` a builder**, even though
   D3 happened -- see D2 for why.
5. **D3 keeps three positional arguments** rather than typestating `state`, because `()` is a
   legitimate `S` and cannot also mean "unset".

---

## H. What was not traded away

Load-bearing, and easy to lose in a refactor this size. All still true:

* **The `Dispatcher` seam.** `conn` depends on the trait, not on `Router`. `tests/dispatch.rs` exists
  to fail if the dependency creeps back. B3 made the *second* seam match it.
* **Tables built once, at startup.** An ID collision is a boot failure, not a runtime error on the
  first client that happens to send the packet.
* **`MakeWith`.** It looks like a wrapper that could be replaced with a blanket
  `impl<F: Fn() -> D> MakeDispatcher for F`, and it cannot: that overlaps the `Arc<Router<S>>` impl
  as far as coherence is concerned.
* **The permit is taken before the accept.** A full server leaves the next connection in the kernel's
  backlog instead of accepting it in order to drop it -- backpressure rather than a refusal the peer
  cannot distinguish from an outage.
* **`CancelOnDrop`.** Dropping the server future -- losing a `select!` against a signal -- would
  otherwise orphan every live connection with nothing able to stop them.
* **Accept errors are never fatal.** The failure that happens in practice is running out of file
  descriptors, which is transient.
