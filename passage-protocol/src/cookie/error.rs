/// The cookie errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Parsing the cookie failed.
    #[error(transparent)]
    ParsingFailed(#[from] serde_json::Error),
}
