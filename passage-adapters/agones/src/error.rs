use std::net::AddrParseError;

/// A [`AgonesError`] is an error thrown by the Agones adapter.
#[derive(thiserror::Error, Debug)]
pub enum AgonesError {
    /// The server has no identifier to uniquely identify it.
    #[error("server has no identifier")]
    NoName,

    /// The server has no address to connect to. An allocated `GameServer` should have an address such
    /// that a user is able to connect to it.
    #[error("server has no address")]
    NoAddress,

    /// The server has no status to determine its state. An allocated `GameServer` should have a status.
    #[error("server {identifier} has no status")]
    NoStatus {
        /// The identifier of the server.
        identifier: String,
    },

    /// The server's ip address could not be parsed. This is likely due to an invalid address format.
    #[error("server {identifier} ip address could not be parsed: {cause}")]
    InvalidAddress {
        /// The identifier of the server.
        identifier: String,
        /// The cause of the error.
        #[source]
        cause: Box<AddrParseError>,
    },

    /// The allocated `GameServer` is not public. Only public `GameServers` can be connected to.
    #[error("server is not public: {identifier}")]
    NotPublic {
        /// The identifier of the server.
        identifier: String,
    },
}
