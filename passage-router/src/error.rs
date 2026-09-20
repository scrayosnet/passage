
pub type Result<T, E = PassageError> = std::result::Result<T, E>;

#[derive(thiserror::Error, Debug)]
pub enum PassageError {
    /// The router failed to build.
    #[error(transparent)]
    Build(#[from] passage_core::router::RouterError),
}
