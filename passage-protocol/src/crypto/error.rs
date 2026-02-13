/// The internal error type for all errors related to the authentication and cryptography.
///
/// This includes errors with the expected passage-packets, packet contents or encoding of the exchanged fields. Errors of the
/// underlying data layer (for Byte exchange) are wrapped from the underlying IO errors. Additionally, the internal
/// timeout limits also are covered as errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An error occurred while reading or writing to the underlying byte stream.
    #[error("error reading or writing data: {0}")]
    IllegalRsa(#[from] rsa::Error),

    #[error("could not encode the public key: {0}")]
    EncodingFailed(#[from] rsa::pkcs8::spki::Error),

    #[error("failed to retrieve randomness: {0}")]
    UnavailableRandom(#[from] rand::rngs::SysError),

    #[error("authentication request failed: {0}")]
    InvalidCipherLength(#[from] cfb8::cipher::InvalidLength),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
