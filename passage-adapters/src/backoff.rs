use rand::{RngExt, SeedableRng};
use serde::Deserialize;
use std::sync::LazyLock;
use tokio::sync::Mutex;

/// A thread-safe random number generator. The generator is seeded with a constant value and should
/// NOT be used for cryptographic algorithms. This is only intended for the backoff jitter where such
/// security is of no concern.
static RNG: LazyLock<Mutex<rand::rngs::SmallRng>> =
    LazyLock::new(|| Mutex::new(rand::rngs::SmallRng::seed_from_u64(0)));

#[derive(Debug, Copy, Clone, Deserialize)]
#[serde(default)]
pub struct ExponentialBackoff {
    pub initial_secs: u64,
    pub max_secs: u64,
    pub max_attempts: u64,
    pub factor: f64,
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
    pub fn once() -> Self {
        Self {
            initial_secs: 0,
            max_secs: 0,
            max_attempts: 1,
            factor: 0.0,
            jitter: 0.0,
        }
    }

    pub async fn secs_after(&self, attempt: u64) -> Option<u64> {
        if attempt >= self.max_attempts {
            return None;
        }
        let secs = self.initial_secs * self.factor.powi(attempt as i32) as u64;
        let jitter = RNG.lock().await.random_range(0.0..self.jitter) as u64;
        Some(secs.min(self.max_secs) + jitter)
    }
}
