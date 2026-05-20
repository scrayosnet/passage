use rand::{RngExt, SeedableRng};
use serde::Deserialize;
use std::sync::LazyLock;
use tokio::sync::Mutex;

/// A thread-safe random number generator. The generator is seeded with a constant value and should
/// NOT be used for cryptographic algorithms. This is only intended for the backoff jitter where such
/// security is of no concern.
static RNG: LazyLock<Mutex<rand::rngs::SmallRng>> =
    LazyLock::new(|| Mutex::new(rand::rngs::SmallRng::seed_from_u64(0)));

/// Configuration for exponential back-off with optional jitter.
///
/// Each successive attempt waits `initial_secs × factor^attempt` seconds, capped at `max_secs`.
/// A random value in `[0, jitter)` is added to avoid synchronised retries. After `max_attempts`
/// the strategy signals that retrying should stop.
#[derive(Debug, Copy, Clone, Deserialize)]
#[cfg_attr(feature = "config-schema", derive(schemars::JsonSchema))]
#[serde(default)]
pub struct ExponentialBackoff {
    /// Wait time in seconds for the first retry.
    pub initial_secs: u64,

    /// Maximum wait time in seconds between retries.
    pub max_secs: u64,

    /// Maximum number of attempts before giving up.
    pub max_attempts: u64,

    /// Multiplicative factor applied to the wait time after each attempt.
    pub factor: f64,

    /// Upper bound of random jitter added to each wait time (seconds).
    pub jitter: f64,
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self {
            initial_secs: 2,
            max_secs: 60,
            max_attempts: 10,
            factor: 2.0,
            jitter: 0.1,
        }
    }
}

impl ExponentialBackoff {
    /// Creates a back-off strategy that permits exactly one attempt with no waiting.
    pub fn once() -> Self {
        Self {
            initial_secs: 0,
            max_secs: 0,
            max_attempts: 1,
            factor: 0.0,
            jitter: 0.0,
        }
    }

    /// Returns the number of seconds to wait before the next attempt, or `None` if `attempt` has
    /// reached or exceeded `max_attempts`.
    pub async fn secs_after(&self, attempt: u64) -> Option<u64> {
        if attempt >= self.max_attempts {
            return None;
        }
        let secs = self.initial_secs * self.factor.powi(attempt as i32) as u64;
        let jitter = RNG.lock().await.random_range(0.0..self.jitter) as u64;
        Some(secs.min(self.max_secs) + jitter)
    }
}
