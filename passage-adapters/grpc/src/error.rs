/// Error indicating that a required field was absent in a gRPC response message.
#[derive(thiserror::Error, Debug)]
#[error("missing data field: {field}")]
pub struct MissingFieldError {
    /// Name of the missing field in the protobuf message.
    pub field: &'static str,
}
