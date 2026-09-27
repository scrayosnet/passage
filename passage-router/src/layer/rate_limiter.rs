use crate::metrics;
use std::collections::HashMap;
use std::hash::Hash;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant};
use tracing::instrument;
use passage_core::router::Layer;

/// [`RateLimiter`] tracks connections per client address over some (approximate) time window.
pub struct RateLimiter<Addr> {
    last_cleanup: Instant,
    buckets: HashMap<Addr, (Instant, f32, f32)>,
    duration: Duration,
    limit: f32,
}

impl<Addr> RateLimiter<Addr>
where
    Addr: Eq + Copy + Hash,
{
    pub fn new(duration: Duration, limit: usize) -> Self {
        assert!(duration.as_secs_f32() > 0f32);
        metrics::rate_limiter_size::set(0u64);
        Self {
            last_cleanup: Instant::now(),
            buckets: HashMap::new(),
            duration,
            limit: limit as f32,
        }
    }

    #[instrument(skip_all)]
    pub fn enqueue(&mut self, key: Addr) -> bool {
        // get the current time only once
        let now = Instant::now();

        // get or insert the bucket
        let (bucket_window, bucket_last, bucket_current) =
            self.buckets.entry(key).or_insert((now, 0f32, 0f32));

        // if the bucket window changed, move bucket counts
        let bucket_age = now.saturating_duration_since(*bucket_window);
        if bucket_age >= self.duration {
            // handle that the last bucket has also expired
            if bucket_age >= 2 * self.duration {
                *bucket_current = 0f32
            }

            // start the next bucket
            *bucket_window = now;
            *bucket_last = *bucket_current;
            *bucket_current = 0f32
        }

        // handle too many visits
        let bucket_last_weight = now.saturating_duration_since(*bucket_window).as_secs_f32()
            / self.duration.as_secs_f32();
        let bucket_value = (*bucket_last * (1f32 - bucket_last_weight)) + *bucket_current;
        if bucket_value >= self.limit {
            return false;
        }

        // update bucket count
        *bucket_current += 1f32;

        // after every second window change, remove all old buckets
        if now.saturating_duration_since(self.last_cleanup) >= self.duration * 2 {
            self.buckets.retain(|_, (last_visit, _, _)| {
                now.saturating_duration_since(*last_visit) < self.duration * 2
            });
            self.last_cleanup = Instant::now();
        }
        metrics::rate_limiter_size::set(self.buckets.len() as u64);

        // allow the request to pass
        true
    }
}

pub struct RateLimiterLayer<Addr> {
    rate_limiter: Option<Mutex<RateLimiter<Addr>>>,
}

impl<Addr> RateLimiterLayer<Addr> where
    Addr: Eq + Hash + Copy,
{
    pub fn new(config: Option<crate::config::RateLimiter>) -> Self {
        Self {
            rate_limiter: config.map(|config| {
                Mutex::new(RateLimiter::new(Duration::from_secs(config.duration), config.limit))
            })
        }
    }
}

impl<Io, Addr> Layer<Io, Addr> for RateLimiterLayer<Addr>
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    Addr: Send + Sync + Eq + Hash + Copy + 'static,
{
    type Io = Io;

    async fn admit(&self, io: Io, addr: Addr) -> Option<(Self::Io, Addr)> {
        let Some(rate_limiter) = self.rate_limiter.as_ref() else {
            return Some((io, addr))
        };

        let mut rate_limiter = rate_limiter.lock().await;
        if !rate_limiter.enqueue(addr) {
            return None;
        }
        Some((io, addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn allow_initial() {
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
    }

    // rejects any request after the window is filled
    #[tokio::test(start_paused = true)]
    async fn reject_many() {
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);

        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(!rate_limiter.enqueue(&0));

        tokio::time::advance(Duration::from_secs(9)).await;

        assert!(!rate_limiter.enqueue(&0));
    }

    #[tokio::test(start_paused = true)]
    async fn allow_after_duration() {
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);

        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(!rate_limiter.enqueue(&0));

        tokio::time::advance(Duration::from_secs(20)).await;

        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(!rate_limiter.enqueue(&0));
    }

    #[tokio::test(start_paused = true)]
    async fn allow_disjoint() {
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);

        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(!rate_limiter.enqueue(&0));

        assert!(rate_limiter.enqueue(&1));
        assert!(rate_limiter.enqueue(&1));
        assert!(rate_limiter.enqueue(&1));
    }
}
