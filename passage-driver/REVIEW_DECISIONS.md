# Review decisions

This file lists the decisions against the REVIEW.md as well as additional changes.

Each section keeps the original decision, followed by **Solved** describing what was actually built
and where. Three decisions were deferred rather than applied; those say so and why.

## A1 + A2

It should create a table for each breaking version, using them as breakpoints (so not every single
protocol version).

Snapshot versions will not be supported for now. These should be rejected similar to how a min version is enforced.

**Solved.** `Packet::id` was a function, so nothing could see *where* a packet changed. It is now
`Packet::IDS`, the same table as data, with `id()` a provided method over it (`src/packet.rs`). That
one change makes the rest fall out:

- `RouterBuilder::build` takes **no version list at all**. It reads the thresholds out of every
  registered packet, sorts and dedupes them, and builds one `Table` per threshold
  (`breakpoints`/`build_table` in `src/router.rs`). `Router::table` binary-searches for the highest
  breakpoint at or below the connection's version, so a release between two thresholds is dispatched
  by the lower one -- which is correct by construction, because nothing about dispatch differs
  between them.
- The floor (`ProtocolVersion::UNKNOWN`) is always a breakpoint, so the version-independent table is
  no longer a special case but the first interval.
- `demo::server::SUPPORTED_VERSIONS` is gone. Nothing in the crate lists protocol versions any more.
- Order became load-bearing, so it is checked: `BuildError::UnorderedIds` rejects a table written
  oldest-first at build time (it would otherwise resolve to an ID from the wrong era, silently, for
  some versions only).

Snapshots: `ProtocolVersion::is_snapshot` (bit 30) and `is_release` (non-negative, not a snapshot)
in `src/version.rs`. Two places act on it. `Router::table` hands anything that is not a release the
floor table, because `at_least` cannot place it -- that is a driver-level guard nobody can forget.
And `demo::server::on_intention` refuses a non-release for login exactly the way it refuses one below
`MIN_LOGIN_VERSION`, so the client is told why instead of being sent a `LoginSuccess` with a field it
cannot parse.

Tests: `a_release_nobody_wrote_down_can_still_log_in` and
`a_snapshot_is_refused_rather_than_treated_as_the_newest_release` (`tests/flow.rs`),
`breakpoints_are_the_versions_the_packets_name` and `an_unordered_id_table_is_a_build_error`
(`src/router.rs`), `a_snapshot_outranks_every_release_and_is_refused_for_it` (`src/version.rs`).

*Not addressed:* the misleading `UnexpectedPacket` diagnostic that made this bug look like a
protocol-ordering error. With per-breakpoint tables the case that produced it cannot happen any
more, and the cross-phase hint is genuinely useful when a client really does send a packet in the
wrong phase, so it was left alone.

## A3

Use the general cancellation token just as recommended.

**Solved.** Every socket write in the loop is now raced against the shutdown token *and* the
lifetime deadline, via `guarded` in `src/conn/connection.rs`. `Connection::write` and
`Connection::flush` return `Result<(), Ending>` -- either the write finished, or the connection is
ending for one of those two reasons. (A first draft hid that behind a `Driven<T>` alias; sitting next
to the crate's own `Result<T, E = Error>` it made two different error types look alike, so it is
spelled out.)

One thing the decision did not cover and the implementation had to: **the final flush cannot use the
same guard.** A cancelled token and an expired deadline are two of the three reasons there is a
disconnect message to write at all, so guarding the last write with them would mean the message
could never be sent. That stretch is bounded by the new
`ConnectionConfig::close_timeout` (default 5 s) instead, via `closing`. The rule is: while running,
cancellation means stop; while ending, only the clock can stop us.

`finish` also now cancels the connection's token *after* the flush rather than before, so the final
write is not racing the signal it just raised.

Test: `a_peer_that_stops_reading_does_not_outlast_its_deadline` (`tests/flow.rs`) -- a 16-byte socket
whose client never reads, which previously survived 600 simulated seconds past a 5-second
`max_lifetime` and past `shutdown.cancel()`.

## A4 + A5

There should be an on_error handler on the dispatcher. It should get the error reason and be able to
send a disconnect packet (cleanup). This should give the handler exclusive rights. This will require
some sort of side channel for the handler to send ops that are handled before any others (in case the
error can be recovered). This is either a return Op or separate priority channel (or whatever you think).

There should also be an Op::Disconnect that tells the connection to close (skipping the on_error handler
as already handled). This will also be used by the on_error handler to close the connection.

However, we have to be able to send multiple ops after each other in a way that no other task may
send ops in between. Probably an Op::Multiple(Vec<Op>) or smallvec

**Solved**, with three deliberate simplifications -- one of which came out of a second review pass
and reversed part of the first attempt (see *Recovery, and why it is gone* below).

`Dispatcher::on_error(ctx, &Ending) -> Result<()>` (`src/conn/dispatch.rs`), with a default body of
`Ok(())` so existing implementations and test doubles are unaffected. `Ending` is
`Failed(Error) | Cancelled | Expired`. On the router side it is registered with
`RouterBuilder::on_error`.

`Connection::serve` is the new shape, and it is straight-line: run the loop, and if it ends for a
reason nobody asked for, **drain the queue first**, then call `on_error`, then drain again, then
report. Draining first is the A4 fix on its own -- "queue a disconnect, then return `Err`" now works
with no `on_error` involved at all. The reported outcome is always the *ending*: `on_error` gets the
last word on the wire, not on the report, and **an error it returns is logged rather than
reported**, because "the apology would not encode" is a detail of the answer and not a second cause.

**`Op::Disconnect` was built and then removed** -- see *Disconnect, and why it is gone* below.
`Op::Close` is the only way a handler ends a connection, and it is also how a handler that has
already sent its own message declines the one `on_error` would add: close, and return `Ok(())`.

`Op::Batch(Vec<Op<S>>)` is the atomic group, built through `ConnectionHandle::batch(|batch| ...)`.
Two properties, both load-bearing for the disconnect case: nothing can interleave with it, and
**nothing is queued at all if building it fails** -- so a message that cannot be encoded does not
leave half an act behind. `Vec` rather than `SmallVec`: a batch is queued once per connection at
most, so the allocation is noise and a dependency would not have earned itself. `Batch` deliberately
omits `set_version` (a batch encodes for the version it was created with, so a change inside one
could only *look* as if it worked) and `fail` (send-then-`Err` already does it).

**Simplification 1: no priority channel, and no returned ops.** Neither turned out to be needed.
Handlers are synchronous and `on_error` runs after the loop has stopped, so nothing the connection
itself polls can queue during it; the only writer that could interleave is a detached task, and
`Op::Batch` already closes that. A second channel would have been a second ordering to reason about
for a case that cannot arise.

**Simplification 2: one operation handler, not two.** The first attempt had a second copy of the
whole `Op` match for the ending path (`settle_op`, ~60 lines), differing only in which clock bounded
a write. It was a standing invitation to add a variant to one and forget the other. `settle` now
calls the same `handle_op` the loop does; the difference is carried entirely by the write guard,
which reads `closing.is_some()`. Arming `closing` *is* the statement "this connection is ending",
which turned an incidental field into the switch that means something.

Also added: `Op::Fail(Error)` and `ConnectionHandle::fail`, for code that has no `Err` to return --
a detached task, or anything else holding a handle outside a handler call.

The demo now has `LoginDisconnect` and an `on_error` that sends it. It only speaks in the login
phase: the configuration-phase disconnect carries a network-NBT text component and `wire` has no NBT
yet. That is a gap in the demo's packet set, and it is called out in the handler's own docs.

### One type per outcome, and the line it draws

`Completion` and `Ending` overlapped: `Ending::Expired` and `Completion::TimedOut` were one event
under two names, `Cancelled` appeared in both, and `serve` ended with a three-line `match` that was a
pure translation between two representations of the same fact.

Fixed in two steps. First `Cancelled` and `TimedOut` moved out of `Completion` into `Ending`. Then
`PeerClosed` followed them, which emptied `Completion` entirely and deleted it: `Outcome::result` is
now `Result<(), Ending>`, and the line it draws is **who decided**.

| | |
|---|---|
| `Ok(())` | a handler closed it |
| `Err(Ending::PeerClosed)` | the peer hung up |
| `Err(Ending::Cancelled)` | the shutdown token |
| `Err(Ending::TimedOut)` | a deadline |
| `Err(Ending::Failed(Error))` | something broke |

Putting a hangup in the `Err` half is not a claim that it went wrong -- `Ending::error()` returns
`None` for three of the four, and `log_completion` still logs a hangup at `debug`. It is a claim that
**we did not finish what we were doing**, and that is the condition `on_error` needs to fire on: a
client that vanishes while its backend is being selected has left a selection running, and releasing
it is the same job as releasing it after a timeout. Before this, a hangup skipped `on_error`
entirely and no handler could catch it -- the connection was gone, so no packet would ever be
dispatched again. `Ending::can_reply()` is how a handler skips the part of that work that needs a
live socket; the cleanup half runs regardless.

The translation `match` is gone -- `serve` ends with `Err(ending)`. `Ending` gained `#[from] Error`,
so the loop's `map_err(Ending::Failed)` calls became plain `?`, and `handle_op` returns
`ControlFlow<()>` instead of an `Option<Completion>` that only ever carried one value. And the split
says something true the old shape did not: a connection killed by a shutdown mid-login used to be
reported as `Ok(Completion::Cancelled)`, so `log_completion` filed it under "connection finished".
It did not finish.

`Dispatcher::on_error` taking `&Ending` now needs no explanation beyond the type: it is called for
exactly the `Err` half, because "we did not end this" and "the dispatcher answers for it" are the
same condition.

Tests: `a_peer_hangup_is_an_ending_but_not_a_failure` (`tests/flow.rs`) and
`a_peer_that_vanishes_mid_flight_still_runs_cleanup` (`tests/ending.rs`).

**`Ending` and `Error` stay separate**, and the boundary is now written into `error.rs` because the
question will come up again. The two merges above removed types that described the same thing twice
-- one event under two names, one mechanism under two names. These describe different things at
different scopes: `Error` is what any fallible operation returns and eleven modules depend on it,
down to `wire.rs` and `codec.rs`; an `Ending` is produced in one place and consumed in one. Merging
would make `Err(Error::Cancelled)` a possible return of `Reader::var_int`, which is the
`Err(ConnectionClosed)` mistake in a new coat, and it would break `Class` -- a hangup, a
cancellation and a deadline have nobody to blame, and a taxonomy of blame with a "nobody" in it is
not one. They meet only at `Ending::Failed(Error)`, which is containment, not duplication.

### Disconnect, and why it is gone

The first attempt added `Op::Disconnect` and `Completion::Disconnected`, justified by "how many did
we turn away". Both were removed: the driver already hands the state back.

`Outcome::state` exists (B5) precisely so a caller can report on a connection, so a handler that
refuses a peer writes `refused: Some("bad_client")` into its own session and the report reads it
there. That is strictly better than a completion variant, because it carries the *reason* -- a
low-cardinality label a metric can be keyed on -- where the variant could only carry the bare fact.
And mechanically the two ops were identical, which is its own answer: an `Op` that does exactly what
another `Op` does, distinguished only by a label the caller can produce better themselves, is not
pulling its weight.

`demo::server` shows the replacement: `on_error` records the label in the same batch as the packet,
and `log_completion` reports refusals separately from ordinary completions. F6 is therefore *not*
addressed after all -- see its section.

### Recovery, and why it is gone

The first attempt had `on_error` return `Recovery::End | Resume`, with `Resume` putting the
connection back in its loop. It was removed on review, for three reasons:

1. **The handler is strictly better placed.** Of the sources that can reach `on_error` with an
   `Ending::Failed`, most are either the failing handler's own `Err` -- which it could simply not
   have raised -- or already covered by `UnknownPolicy::Ignore`. The rest are a broken transport, our
   own encoding bugs, and the read gate, none of which mean anything to resume. `on_error` sees an
   `Error` and a phase; the handler saw the packet and the state.
2. **It was unsound.** `settle` deliberately swallows write failures, on the grounds that the reason
   for ending has already been decided. That is only true if the connection *is* ending. On the
   `Resume` path it carried on after having silently dropped a failed write.
3. **It cost more than it looked.** A public type in the trait signature every implementor had to
   return, a `loop` in `serve` that existed for nothing else, and a `self.closing = None` reset that
   was pure state-juggling.

`on_error` returning `Result<()>` and being logged-not-reported falls out of the same reasoning: if
it cannot veto the ending, there is nothing for its error to mean.

Tests: `tests/ending.rs` (7 tests) -- a refused login is told why, a shutdown still gets a message
out, what a handler queued before failing is written, batches are atomic and all-or-nothing, a
handler decides for itself what is survivable (with `UnknownPolicy::Ignore` alongside it), and an
answer that cannot be sent does not replace the reason.

## B1 + B2

Each phase packet will be its own struct. That's ok. Maybe we can remove the check in the connection then.

**Solved.** The check is gone. `Op::Send` no longer carries a phase and
`InternalError::StaleEncoding` no longer has `encoded_phase`/`phase` -- it is now purely the version
guard, which *is* a snapshot comparison. The phase half never was: `Packet::PHASE` is a constant, so
it compared the packet's identity against the connection rather than a snapshot against the present.
The error's docs now say why there is no phase counterpart.

Test: `the_phase_a_packet_belongs_to_is_its_own_not_the_connections` (`tests/flow.rs`) replaces
`a_packet_encoded_for_a_phase_the_connection_left_is_refused`, and records the reasoning.

## B3, B4, B5, B9, B10

Apply recommendations

**Solved, all five.**

- **B3** -- `ConnectionHandle::spawn`'s docs now say plainly that the future is polled *by the
  connection loop* and that one which blocks stops the connection. `detach` is the new escape hatch:
  a real `tokio::spawn`, talking back through a cloned handle, with an error routed to `Op::Fail`.
  What it cannot do is hold the read gate, and that is documented -- `exclusive` counts tasks the
  connection polls, and a detached one is not. It returns `()` rather than `Result<()>`, because
  nothing at that end can fail and a `Result` that is always `Ok` is a lie in a signature.
- **B4** -- `serve` runs each connection on its own `tokio::spawn` inside the tracked task and awaits
  the handle, so a panic becomes an `Error::internal("panic", ..)` and reaches `on_finish` like any
  other outcome. It used to unwind straight past the report and vanish.
  Test: `a_panicking_connection_is_reported_rather_than_vanishing` (`tests/server.rs`).
- **B5** -- `Connection::run` now returns `Outcome<S> { result, state, version, phase }`, and
  `on_finish` receives `Finished { elapsed, addr, outcome }` with `state()`, `completion()` and
  `error()` helpers. Everything the old implementation's metrics needed -- duration, peer, handshake
  intent, locale, view distance -- is reachable from it. `demo::server::log_completion` was rewritten
  to use them.
- **B9** -- `serve` is generic over `MakeDispatcher<S>`, implemented for `Arc<Router<S>>` and for
  closures via `make_with`. The cost is `Arc::new(router()?)` at the call site, which is honest: it
  puts "built once, shared by every connection" where you can see it.
- **B10** -- the accept loop's `live` token is held in a `CancelOnDrop` guard, so dropping the server
  future (a `select!` that loses to a signal, say) cancels the connections instead of orphaning them.

## B6

Will be added later

**Not applied**, as decided. Nothing was added for spans, metrics or packet-level instrumentation.
Worth noting for when it is: `Dispatcher` being a trait makes an inbound decorator
(`Instrumented<RouterDispatcher<S>>`) possible today, but it sees `(id, payload)` rather than the
decoded packet, and there is still no seam at all on the outbound path -- encoding happens inside
`ConnectionHandle::send`. A symmetric story needs both.

## B7, B8, F3

Maybe we could introduce a middelware/interceptor pattern that handles this. Both on the server and connection.

**Partly solved; the general pattern deferred.**

The two concrete problems are fixed, because both were about *where* code runs rather than about
composition:

- **B7** -- `Listener::prepare(io, addr)` is a new associated function with a default no-op, called
  on the connection's own task rather than in the accept loop. That is where a PROXY header or a TLS
  handshake belongs, and it may return a different address, which is the point for PROXY. The
  previous suggestion -- put it in `accept` -- is gone from the docs, along with the misleading
  claim; awaiting it there would have let one silent peer hold up every accept.
- **B8** -- `Server::max_connections(n)` takes its permit *before* the accept, so a full server
  leaves connections in the backlog instead of accepting them to refuse them. `Server::on_accept`
  decides admission on the connection's task, after `prepare`, so a rate limiter sees the address a
  PROXY header reported.
  Test: `a_refused_peer_never_reaches_the_protocol` (`tests/server.rs`).

**Deferred: the middleware/interceptor pattern itself.** Building it properly means an
`Io`-changing, composable chain on the server *and* a two-sided seam on the connection (F3's real
difficulty is the outbound half, which has no seam at all). Doing that well is its own change, it
overlaps heavily with B6 -- which is explicitly "later" -- and guessing at its shape now would most
likely produce something that has to be redone once the telemetry requirements are concrete. The
hooks above cover what B7 and B8 actually needed in the meantime.

## F1

The handler will handle that itself (uses state)

**Solved as documentation plus a worked example**, since the decision is to keep the mechanism as it
is. `conn`'s module docs gained a section, "Sequence is the handler's business", saying outright that
the connection knows phases and not steps and that nothing enforces order within a phase. The demo
now shows the shape: `on_login_start` refuses a second login and `on_login_acknowledged` refuses one
that never started, both by checking `ctx.state`.

## F2

Ignore for now.

**Not applied**, as decided. No compression.

## F4

We can probably remove the direction limitation for now and handle the rest later.

**Solved.** `Router::builder()` takes no `Direction`, `BuildError::WrongDirection` is gone,
`Router::inbound` is gone (it was a dead accessor anyway), and `ProtocolError::UnknownPacket` no
longer carries a direction it could not obtain. The review's other two dead accessors went with it:
`Router::unknown_policy`, and the `Router::breakpoints` this change set had briefly added on
speculation -- `Debug for Router` already prints the table count, which was its only real use. `Packet::DIRECTION` stays: it is still part of a
packet's identity and still documents which way it travels.

The table is keyed by phase and ID alone, so registering both directions of one phase collides on
the IDs they share. That is left as-is and documented on `Router::builder` -- it is the honest signal
that a proxy wants a direction-keyed table, rather than a silent preference for one half of the
protocol.

Test: `a_router_is_not_bound_to_one_direction` (`tests/flow.rs`), which builds a clientbound router
and then shows the collision.

## F5

We could enforce a state trait that names a mutation trait. The caller is then able to provider their
own fast implementation. The fallabck would be the boxed fn.

```rs
pub trait State {
    type Mutator<State>;
}

pub trait Mutator<S> {
    fn apply(self, state: &mut <S>);
}

// example:
pub enum MyMutator<MyState> {
    Increment,
    Decrement,
    Other(Box<dyn FnOnce(&mut S) + Send>)
}

impl Mutator<MyState> for MyMutator<MyState> {
    fn apply(self, state: &mut MyState) {
        match self {
            Increment => state.count += 1,
            Decrement => state.count -= 1,
            Other => self.0(state),
        }
    }
}
```

**Deferred, deliberately.** Two reasons, and I would rather flag them than guess:

1. **It buys nothing yet and costs every state type.** The allocation it removes is one `Box` per
   state change -- a handful per Passage connection, against a TCP handshake and a session-server
   round trip. In exchange, every `S` needs an `impl State` with an associated mutation type before
   it can be used at all, including `()` in tests. That trade only turns positive in the play phase,
   which nothing here reaches.
2. **The sketch needs a design pass first.** `type Mutator<State>` is a GAT whose parameter shadows
   the trait's own name, and `MyMutator<MyState>` is parameterised by the state it already names.
   The shape that works is closer to:

   ```rust
   pub trait Mutate<S> { fn apply(self, state: &mut S); }

   pub trait State: Send + 'static {
       /// `BoxedMutation<Self>` is the general answer; override for a fast path.
       type Mutation: Mutate<Self> + Send + 'static;
       /// Keeps `update(|s| ..)` working for every state, fast path or not.
       fn mutation(f: impl FnOnce(&mut Self) + Send + 'static) -> Self::Mutation;
   }
   ```

   with `Op::With(S::Mutation)` and a new `ConnectionHandle::mutate` for the typed path. That is a
   change to `Op`, the handle, `Ctx`, `Connection`, the demo and every test, and it is worth doing
   once the play phase is real rather than twice.

Say the word and I will do it; nothing else in this change set depends on the outcome.

## F6

Ignore for now?

**Not applied**, as decided -- after a detour. A first attempt added `Completion::Disconnected` for
exactly this, and it was removed on review: the distinction belongs in the connection state, where it
can carry the reason, not in a driver-level enum that can only carry the fact. `demo::server` now
demonstrates that (`Session::refused`), which is the shape a real answer to F6 would take. The driver
itself still reports one `Closed` for every deliberate ending.

## O1

Remove the idle handler for now. This can be handled by the tick handler implementation or so. Lets
keep it simple for now.

**Solved.** `ConnectionConfig::max_idle`, the `idle` sleep and `reset_idle` are gone, and the loop's
deadline arm is down to one. `tick_interval`'s docs now point out that a read deadline belongs in the
tick handler, which sees the phase and the state and can therefore say "nothing has arrived and we
are still waiting for the handshake" -- something the connection could not.

`an_idle_connection_is_dropped_by_its_deadline` was replaced by
`a_peer_that_stops_reading_does_not_outlast_its_deadline` (see A3), which tests the harder half of
what the idle deadline was standing in for.

---

## Not covered by any decision

Sections **C** (limits, defaults, hardening) and **D** (smaller things) of `REVIEW.md` had no
decisions, and were left alone -- with two exceptions that A3 and O1 forced:

- `ConnectionConfig::close_timeout` was added with a non-`None` default (5 s), because A3's fix needs
  a bound on the final write. It is the only deadline that defaults to on. C4's wider point --
  that `tick_interval` and `max_lifetime` still default to `None`, so a default `serve` has no
  timeout at all -- stands.
- C1 (an 8 KiB frame limit smaller than the demo's own 32 KiB field limits, and smaller than the old
  implementation's 10,000) is untouched, and still means a favicon-carrying status response would be
  refused as an internal error.

## Verification

`cargo test`: 73 tests across the library and five integration binaries, plus doctests -- all
passing. `cargo fmt --all --check`, `cargo clippy --all-targets --all-features` (0 warnings) and
`cargo doc --no-deps` (0 warnings) are clean.
