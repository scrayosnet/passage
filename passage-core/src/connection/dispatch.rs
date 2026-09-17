use crate::connection::{ConnectionError, Ctx};

/// The [`Dispatcher`] is a thin wrapper around custom handler functions. These return custom errors
/// that cannot be predicted at this point. As such, it refers to [`anyhow::Error`] instead.
pub type DispatchError = anyhow::Error;

/// The [`Dispatcher`] is a thin wrapper around custom handler functions. These return custom errors
/// that cannot be predicted at this point. As such, it refers to [`anyhow::Error`] instead.
type Result<T, E = DispatchError> = std::result::Result<T, E>;

/// A [`Dispatcher`] is used by the connection to handle incoming packets. It is implemented for [`Box`]
/// and [`Option`].
///
/// The [`Dispatcher`] is a thin wrapper around custom handler functions returning [`DispatchError`].
pub trait Dispatcher<S> {
    /// Called when the connection changes the protocol version. This can be used to update internal
    /// dispatch tables.
    fn on_version(&mut self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Handles an incoming packet.
    fn on_frame(&self, ctx: Ctx<'_, S>, id: i32, payload: &[u8]) -> Result<()> {
        let _ = (ctx, id, payload);
        Ok(())
    }

    /// Handles a tick event.
    fn on_tick(&self, ctx: Ctx<'_, S>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Handles a connection error. It is called before the connection is closed and should be used
    /// to send custom disconnect packets to the peer.
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

/// [`MakeDispatcher`] is a builder for [`Dispatcher`]s. In general, a connection [`Dispatcher`] is
/// stateful. This builder allows the driver to create a new dispatcher for each request.
pub trait MakeDispatcher<S>: Send + 'static {
    /// The dispatcher this produces.
    type Dispatcher: Dispatcher<S> + Send + 'static;

    /// Makes one, for one connection.
    fn make(&self) -> Self::Dispatcher;
}
