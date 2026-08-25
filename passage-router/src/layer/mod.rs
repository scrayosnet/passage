//! Everything that happens to a socket before the protocol takes over.
//!
//! The layers are stacked in the order they are listed here: the PROXY header has to be read off
//! the socket before anything else looks at it, the rate limiter needs the address that header
//! carries to limit the right peer, and the metrics layer counts what the two of them let through.

mod metrics;
mod proxy_protocol;
mod rate_limiter;

pub use metrics::{MetricsLayer, Tracked};
pub use proxy_protocol::{ProxyProtocol, ProxyProtocolLayer};
pub use rate_limiter::{RateLimiter, RateLimiterLayer};
