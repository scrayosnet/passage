use crate::connection::{ConnectionError, Ctx, Result};

/// The dispatcher is used by the connection to handle incoming packets.
pub trait Dispatcher<S> {
    /// Handles an incoming packet.
    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let _ = (ctx, id, payload);
        Ok(())
    }

    /// Handles a tick event. This is called independent of the current connection phase and may be
    /// disabled using the [`Dispatcher::ticks`] getter.
    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Handles a connection error before the connection is closed. After the handler completes, the
    /// connection is dropped. This should be used to send a disconnect packet to the peer (if possible).
    ///
    /// It collects errors from both the connection implementation and custom handlers.
    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        let _ = (ctx, error);
        Ok(())
    }
}

impl<S> Dispatcher<S> for () {}

impl<S, D: Dispatcher<S> + ?Sized> Dispatcher<S> for Box<D> {
    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        (**self).on_frame(ctx, id, payload)
    }

    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        (**self).on_tick(ctx)
    }

    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        (**self).on_error(ctx, error)
    }
}

impl <S, D: Dispatcher<S>> Dispatcher<S> for Option<D> {
    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_frame(ctx, id, payload)
    }

    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_tick(ctx)
    }

    fn on_error(&self, ctx: Ctx<'_, S>, error: &mut ConnectionError) -> Result<()> {
        let Some(this) = self else {
            return Ok(());
        };
        this.on_error(ctx, error)
    }
}
