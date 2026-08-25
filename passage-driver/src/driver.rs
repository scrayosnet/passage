use futures::channel::mpsc;
use futures::StreamExt;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use crate::error::DriverError;
use crate::flow::Flow;
use crate::hooks::{Ctx, Hooks, Op};

pub struct Driver<H> {
    /// The packet frame codec. It decodes the TCP stream into packet frames independent of the current
    /// phase and protocol version.
    framed: mpsc::UnboundedReceiver<Result<(), DriverError>>,

    /// The channel to receive operations from the hooks.
    ops: mpsc::UnboundedReceiver<Op>,

    /// The cancellation token. When the token is canceled, the driver is shut down.
    shutdown: CancellationToken,

    /// The set of pending hooks. This is used to wait for all hooks to complete while processing and
    /// before shutting down.
    pending_hooks: JoinSet<Result<(), DriverError>>,

    /// The hooks of the driver. They are called for incoming packets and on tick.
    hooks: H,

    /// The keep alive interval for the ticker.
    ticker: tokio::time::Interval,
}

impl <S, H: Hooks<S>> Driver<H> {
    fn ctx(&mut self) -> Ctx<S> {
        todo!()
    }

    async fn handle_ops(&mut self, ops: Option<Op>) -> Result<(), DriverError> {
        // TODO implement me!
        Ok(())
    }

    // TODO maybe make a hook?
    async fn handle_error(&mut self, err: DriverError) -> Result<(), DriverError> {
        Ok(())
    }

    // TODO does the lifetime make sense here?
    fn handle_flow(&mut self, flow: Flow<'static, Result<(), DriverError>>) -> Result<(), DriverError> {
        match flow {
            Flow::Ready(result) => result,
            Flow::Pending(future) => {
                self.pending_hooks.spawn(future);
                Ok(())
            },
        }
    }

    pub async fn listen(&mut self) -> Result<(), DriverError> {
        loop {
            tokio::select! {
                biased;
                // First, any pending operations are handled.
                ops = self.ops.recv() => {
                    let result = self.handle_ops(ops).await;
                    if let Err(err) = result {
                        self.handle_error(err).await?;
                        break;
                    }
                },

                // Then, any async tasks that sent these operations are joined. This frees memory.
                result = self.pending_hooks.join_next() => {
                    if let Err(err) = result {
                        self.handle_error(err).await?;
                        break;
                    }
                },

                // Then, if the connection is shutdown, then the loop is exited.
                _ = self.shutdown.cancelled() => {
                    self.handle_error(DriverError::Timeout).await?;
                    break;
                },

                // Then, the ticks are handled to ensure that keep alive packets are sent.
                tick = self.ticker.tick() => {
                    let flow = self.hooks.on_tick(&self.ctx())
                    let result = self.handle_flow(flow);
                    if let Err(err) = result {
                        self.handle_error(err).await?;
                        break;
                    }
                },

                // Finally, the next packet is read from the stream.
                frame = self.framed.next() => {
                    // Stop all on connection close. There is nothing we can do about it.
                    let Some(frame) = frame else {
                        self.shutdown.cancel();
                        break;
                    };

                    // Handle the error
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(err) => {
                            self.handle_error(err).await?;
                            break;
                        }
                    }

                    // Dispatch the next frame
                    let ctx = self.ctx();
                    let flow = self.hooks.on_handshake_intention(&ctx, frame);
                    let result = self.handle_flow(flow);
                    if let Err(err) = result {
                        self.handle_error(err).await?;
                        break;
                    }
                },
            };
        };
        Ok(())
    }
}
