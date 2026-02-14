use std::net::AddrParseError;

#[derive(thiserror::Error, Debug)]
pub enum GameServerError {
    #[error("server has no identifier")]
    NoName,

    #[error("server {identifier} has no status")]
    NotStatus {
        /// The identifier of the server.
        identifier: String,
    },

    #[error("server {identifier} ip address could not be parsed: {cause}")]
    InvalidAddress {
        /// The identifier of the server.
        identifier: String,
        /// The cause of the error.
        #[source]
        cause: Box<AddrParseError>,
    },

    #[error("server is not public: {identifier}")]
    NotPublic {
        /// The identifier of the server.
        identifier: String,
    },
}
