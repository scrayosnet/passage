use std::collections::HashMap;
use std::hash::Hash;
use tokio::time::{Duration, Instant};

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
        Self {
            last_cleanup: Instant::now(),
            buckets: HashMap::new(),
            duration,
            limit: limit as f32,
        }
    }

    pub fn enqueue(&mut self, key: T) -> bool {
        // get current time once
        let now = Instant::now();

        // get or insert bucket
        let (bucket_window, bucket_last, bucket_current) =
            self.buckets.entry(key).or_insert((now, 0f32, 0f32));

        // if bucket window changed, move bucket counts
        if now.saturating_duration_since(*bucket_window) >= self.duration {
            *bucket_window = Instant::now();
            *bucket_last = *bucket_current;
            *bucket_current = 0f32
        }

        // handle too many visits
        let bucket_index = now.saturating_duration_since(*bucket_window).as_secs_f32()
            / self.duration.as_secs_f32();
        let bucket_value =
            (*bucket_last * (1f32 - bucket_index)) + (*bucket_current * bucket_index);
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
