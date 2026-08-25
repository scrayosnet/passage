/// The cookie errors.
#[derive(thiserror::Error, Debug)]
pub enum CookieError {
    #[error("The cookie requires a secret to be present.")]
    SecretRequired,

    /// Parsing the cookie failed.
    #[error(transparent)]
    ParsingFailed(#[from] serde_json::Error),
}
