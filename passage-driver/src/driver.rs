//! The driver: the only thing that owns the socket.
//!
//! The driver does four things and nothing else -- frame, dispatch, tick, and shut down. All
//! protocol logic lives in the [`Router`]'s handlers.
//!
//! # The loop
//!
//! ```text
//! biased select:
//!   1. queued operations   (writes, encryption switch, close)   <- drained first
//!   2. the pending handler (if the last handler returned Pending)
//!   3. detached tasks      (joined so failures surface)
//!   4. shutdown
//!   5. tick                (keep-alives, deadlines)
//!   6. the next frame      (only while no handler is pending)
//! ```
//!
//! The priority order is the design. Operations first means a handler's writes reach the socket
//! before the next packet is even looked at. Reading last, and only when nothing is pending, is what
//! gives a connection **ordering and backpressure for free**: at most one handler for a given
//! connection runs at a time, so two packets can never race to answer each other, and a slow
//! handler stops us from buffering more input.
//!
//! Work that genuinely has to overlap with further traffic is explicit:
//! [`ConnHandle::detach`](crate::conn::ConnHandle::detach). That is the difference between "the
//! framework decided to run my handlers concurrently" and "I asked for concurrency here".

use crate::codec::{Frame, FrameCodec};
use crate::conn::{ConnHandle, Ctx, Op};
use crate::error::{Class, InternalError, Result};
use crate::flow::{Flow, Outcome, Update};
use crate::packet::Phase;
use crate::router::{Bound, Dispatch, Router};
use crate::version::ProtocolVersion;
use crate::wire::Limits;
use futures::future::BoxFuture;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinSet;
use tokio_util::codec::Framed;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace, warn};

/// How a connection ended.
///
/// Ending is not an error. Distinguishing *how* it ended is what lets the caller log a scanner
/// hang-up at `debug` and a decoding bug at `warn` without inspecting error variants.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Completion {
    /// A handler asked to close the connection (status response sent, transfer sent, disconnect).
    Closed,

    /// The peer hung up.
    PeerClosed,

    /// The connection was cancelled -- shutdown or timeout.
    Cancelled,
}

/// Static configuration of a connection.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    /// The decoding limits.
    pub limits: Limits,

    /// How often the tick handler runs, if at all.
    pub tick_interval: Option<Duration>,

    /// The protocol version before the handshake is processed.
    pub initial_version: ProtocolVersion,

    /// The phase the connection starts in.
    pub initial_phase: Phase,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            tick_interval: None,
            initial_version: ProtocolVersion::UNKNOWN,
            initial_phase: Phase::Handshake,
        }
    }
}

/// Drives one connection.
pub struct Driver<S, T> {
    framed: Framed<T, FrameCodec>,
    router: Arc<Router<S>>,
    bound: Bound<S>,
    state: S,
    handle: ConnHandle<S>,
    ops: tokio::sync::mpsc::UnboundedReceiver<Op<S>>,
    pending: Option<BoxFuture<'static, Result<Update<S>>>>,
    detached: JoinSet<Result<Update<S>>>,
    ticker: Option<tokio::time::Interval>,
    shutdown: CancellationToken,
    config: DriverConfig,
}

/// What the select in the main loop produced.
enum Step<S> {
    Op(Option<Op<S>>),
    Pending(Result<Update<S>>),
    Detached(Option<std::result::Result<Result<Update<S>>, tokio::task::JoinError>>),
    Tick,
    Shutdown,
    Frame(Option<Result<Frame>>),
}

impl<S, T> Driver<S, T>
where
    S: Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin,
{
    /// Creates a driver for `io`, together with the handle for its connection.
    ///
    /// The handle is returned so the caller can talk to the connection from the outside -- close it,
    /// or hand it to something that will. Handlers get their own copy through
    /// [`Ctx`](crate::conn::Ctx).
    pub fn new(
        io: T,
        router: Arc<Router<S>>,
        state: S,
        config: DriverConfig,
        shutdown: CancellationToken,
    ) -> Result<(Self, ConnHandle<S>)> {
        let bound = router.bind(config.initial_version)?;
        let (handle, ops) = ConnHandle::new(
            config.initial_version,
            config.initial_phase,
            shutdown.clone(),
        );

        let ticker = config.tick_interval.map(|interval| {
            let mut ticker =
                tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            ticker
        });

        let driver = Self {
            framed: Framed::new(io, FrameCodec::new(config.limits)),
            router,
            bound,
            state,
            handle: handle.clone(),
            ops,
            pending: None,
            detached: JoinSet::new(),
            ticker,
            shutdown,
            config,
        };
        Ok((driver, handle))
    }

    /// Runs the connection to completion.
    ///
    /// Returns how it ended, or the error that ended it. Peer errors are returned like any other:
    /// the caller decides the log level from [`Error::class`].
    pub async fn run(mut self) -> Result<Completion> {
        let completion = loop {
            let step = tokio::select! {
                biased;

                // 1. Everything handlers asked for, before anything else.
                op = self.ops.recv() => Step::Op(op),

                // 2. The one in-flight handler, if any.
                result = async { self.pending.as_mut().expect("pending is set").await },
                    if self.pending.is_some() => Step::Pending(result),

                // 3. Detached tasks, so their failures are not silently dropped.
                joined = self.detached.join_next(), if !self.detached.is_empty() => {
                    Step::Detached(joined)
                },

                // 4. Cancellation.
                () = self.shutdown.cancelled() => Step::Shutdown,

                // 5. Ticks, but not while a handler is pending -- a keep-alive queued behind a
                //    stalled handler is worse than a late one.
                _ = async { self.ticker.as_mut().expect("ticker is set").tick().await },
                    if self.ticker.is_some() && self.pending.is_none() => Step::Tick,

                // 6. Input, only when nothing is in flight. This is the backpressure.
                frame = self.framed.next(), if self.pending.is_none() => Step::Frame(frame),
            };

            match step {
                Step::Op(None) => break Completion::Closed,
                Step::Op(Some(op)) => {
                    if let Some(completion) = self.handle_op(op).await? {
                        break completion;
                    }
                }
                Step::Pending(result) => {
                    self.pending = None;
                    // Applied here, with exclusive access, before the next frame is read.
                    result?.run(&mut self.state);
                }
                Step::Detached(joined) => self.handle_joined(joined)?,
                Step::Shutdown => break Completion::Cancelled,
                Step::Tick => self.handle_tick()?,
                Step::Frame(None) => break Completion::PeerClosed,
                Step::Frame(Some(frame)) => self.handle_frame(frame?)?,
            }
        };

        self.finish().await;
        Ok(completion)
    }

    /// Carries out one queued operation. Returns a completion if the connection should end.
    async fn handle_op(&mut self, op: Op<S>) -> Result<Option<Completion>> {
        match op {
            Op::Send(encoded) => {
                trace!(packet = encoded.name, "writing packet");
                self.framed.send(encoded).await?;
            }
            Op::Encrypt(cipher) => {
                debug!("enabling encryption");
                self.framed.codec_mut().set_cipher(cipher);
            }
            Op::Detach(future) => {
                self.detached.spawn(future);
            }
            Op::Flush(waiter) => {
                self.framed.flush().await?;
                // The waiter having gone away is fine: it only means nobody is listening anymore.
                let _ = waiter.send(());
            }
            Op::Close => {
                self.framed.flush().await?;
                return Ok(Some(Completion::Closed));
            }
        }
        Ok(None)
    }

    fn handle_joined(
        &mut self,
        joined: Option<std::result::Result<Result<Update<S>>, tokio::task::JoinError>>,
    ) -> Result<()> {
        match joined {
            // The set was drained between the guard and the poll; nothing to do.
            None => Ok(()),
            Some(Ok(result)) => {
                result?.run(&mut self.state);
                Ok(())
            }
            Some(Err(err)) if err.is_cancelled() => Ok(()),
            // A panicking handler must not take the process with it, but it is our bug: surface it
            // as internal so it is logged loudly and reported.
            Some(Err(err)) => Err(InternalError::Handler(Box::new(err)).into()),
        }
    }

    fn handle_tick(&mut self) -> Result<()> {
        let Some(handler) = self.router.tick_handler().cloned() else {
            return Ok(());
        };
        // The borrow of `state` has to end before `accept_flow` takes `&mut self` again.
        let flow = {
            let ctx = Ctx::new(&mut self.state, &self.handle, self.config.limits);
            handler.call(ctx)
        };
        self.accept_flow(flow)
    }

    fn handle_frame(&mut self, frame: Frame) -> Result<()> {
        // The version is only ever set once, by the handshake handler. Rebinding here keeps that
        // out of the handler's hands: it sets a number, the driver picks the matching table.
        let version = self.handle.version();
        if self.bound.version() != version {
            debug!(%version, "binding dispatch table");
            self.bound = self.router.bind(version)?;
        }

        let phase = self.handle.phase();
        let ctx = Ctx::new(&mut self.state, &self.handle, self.config.limits);
        let dispatch = self.bound.dispatch(
            ctx,
            phase,
            self.router.inbound(),
            frame.id,
            &frame.payload,
            self.router.unknown_policy(),
        )?;

        match dispatch {
            Dispatch::Handled { name, flow } => {
                trace!(packet = name, ?phase, "dispatched packet");
                self.accept_flow(flow)
            }
            Dispatch::Ignored => {
                trace!(id = frame.id, ?phase, "ignoring unhandled packet");
                Ok(())
            }
        }
    }

    /// Takes a handler's flow: finish it now, or park it as the pending handler.
    fn accept_flow(&mut self, flow: Outcome<S>) -> Result<()> {
        match flow {
            Flow::Ready(result) => {
                result?.run(&mut self.state);
                Ok(())
            }
            Flow::Pending(future) => {
                debug_assert!(
                    self.pending.is_none(),
                    "a handler was dispatched while another was pending",
                );
                self.pending = Some(future);
                Ok(())
            }
        }
    }

    /// Ends the connection: stop everything we started, then let the socket go.
    async fn finish(mut self) {
        self.shutdown.cancel();
        self.detached.shutdown().await;
        if let Err(err) = self.framed.flush().await {
            debug!(cause = %err, "failed to flush on close");
        }
        if let Err(err) = self.framed.close().await {
            debug!(cause = %err, "failed to close the socket");
        }
    }
}

/// Logs a finished connection at the level its outcome deserves.
///
/// This is the piece that the previous implementation could not express: `Err(ConnectionClosed)`
/// meant both "done" and "broken", so every call site had to special-case it and any new error
/// variant silently fell into the wrong bucket.
pub fn log_completion(result: &Result<Completion>) {
    match result {
        Ok(completion) => debug!(?completion, "connection finished"),
        Err(err) => match err.class() {
            Class::Peer | Class::Transport => {
                debug!(cause = %err, kind = err.label(), "connection dropped")
            }
            Class::Internal => {
                warn!(cause = %err, kind = err.label(), "connection failed")
            }
        },
    }
}
