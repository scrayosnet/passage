use futures::future::BoxFuture;

/// A [`Flow`] is a sync-/asynchronous result. It either directly returns its result or a future for
/// it. This helper is used for methods that may need to return a [`Future`] but generally don't. This
/// allows an API to support both.
#[must_use]
pub enum Flow<'a, T> {
    /// The ready result.
    Ready(T),

    /// The pending result future.
    Pending(BoxFuture<'a, T>),
}

impl<'a, T> Flow<'a, T> {
    /// Creates a new [`Flow`] that is ready with the given value.
    pub fn ready(value: T) -> Self { Flow::Ready(value) }

    /// Creates a new [`Flow`] that is pending with the given future.
    pub fn later(value: impl Future<Output = T> + Send + 'a) -> Self {
        Flow::Pending(Box::pin(value))
    }
}

impl<'a, T> From<T> for Flow<'a, T> {
    fn from(value: T) -> Self { Flow::Ready(value) }
}
