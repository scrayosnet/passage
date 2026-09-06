# Review decisions

This file lists the decisions against the REVIEW.md as well as additional changes.

## A1 + A2

It should create a table for each breaking version, using them as breakpoints (so not every single
protocol version).

Snapshot versions will not be supported for now. These should be rejected similar to how a min version is enforced.

## A3

Use the general cancellation token just as recommended.

## A4 + A5

There should be an on_error handler on the dispatcher. It should get the error reason and be able to
send a disconnect packet (cleanup). This should give the handler exclusive rights. This will require
some sort of side channel for the handler to send ops that are handled before any others (in case the
error can be recovered). This is either a return Op or separate priority channel (or whatever you think).

There should also be an Op::Disconnect that tells the connection to close (skipping the on_error handler
as already handled). This will also be used by the on_error handler to close the connection.

However, we have to be able to send multiple ops after each other in a way that no other task may
send ops in between. Probably an Op::Multiple(Vec<Op>) or smallvec

## B1 + B2

Each phase packet will be its own struct. That's ok. Maybe we can remove the check in the connection then.

## B3, B4, B5, B9, B10

Apply recommendations

## B6

Will be added later

## B7, B8, F3

Maybe we could introduce a middelware/interceptor pattern that handles this. Both on the server and connection.

## F1

The handler will handle that itself (uses state)

## F2

Ignore for now.

## F4

We can probably remove the direction limitation for now and handle the rest later.

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

## F6

Ignore for now?


## O1

Remove the idle handler for now. This can be handled by the tick handler implementation or so. Lets
keep it simple for now.
