use std::collections::HashMap;
use std::hash::Hash;
use tokio::time::{Duration, Instant};
use tracing::instrument;

/// [`RateLimiter`] tracks connections per client address over some (approximate) time window.
pub struct RateLimiter<T> {
    last_cleanup: Instant,
    buckets: HashMap<T, (Instant, f32, f32)>,
    duration: Duration,
    limit: f32,
}

impl<T> RateLimiter<T>
where
    T: Eq + Copy + Hash,
{
    pub fn new(duration: Duration, limit: usize) -> Self {
        assert!(duration.as_secs_f32() > 0f32);
        Self {
            last_cleanup: Instant::now(),
            buckets: HashMap::new(),
            duration,
            limit: limit as f32,
        }
    }

    #[instrument(skip_all)]
    pub fn enqueue(&mut self, key: T) -> bool {
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

        true
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
