#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ParsingFailed(#[from] serde_json::Error),
}
