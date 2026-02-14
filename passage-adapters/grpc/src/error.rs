#[derive(thiserror::Error, Debug)]
#[error("missing data field: {field}")]
pub struct MissingFieldError {
    pub field: &'static str,
}
