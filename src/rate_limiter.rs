use crate::metrics::{RATE_LIMITER, RateLimiterLabels};
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use tokio::time::{Duration, Instant};

/// [`RateLimiter`] tracks connections per client address over some time window.
///
/// The limiter automatically cleans itself up if it gets too large.
pub(crate) struct RateLimiter<T> {
    entries: HashMap<T, VecDeque<Instant>>,
    duration: Duration,
    entry_max_size: usize,
    size: usize,
}

impl<T> RateLimiter<T>
where
    T: Eq + Copy + Hash,
{
    pub(crate) fn new(duration: Duration, size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            duration,
            entry_max_size: size,
            size: 0,
        }
    }

    /// Enqueues for a given key. If the rate limit is reached, it returns
    /// false.
    pub(crate) fn enqueue(&mut self, key: &T) -> bool {
        // handle zero sized rate limiter
        if self.entry_max_size < 1 {
            return false;
        }

        // check whether key is already registered, otherwise add
        let Some(value) = self.entries.get_mut(key) else {
            self.entries
                .insert(*key, VecDeque::from_iter([Instant::now()]));
            self.size += 1;
            RATE_LIMITER
                .get_or_create(&RateLimiterLabels {})
                .set(self.size as i64);
            return true;
        };

        // clear non-recent entries
        while let Some(front) = value.front() {
            if front.elapsed() <= self.duration {
                break;
            }
            value.pop_front();
            self.size -= 1;
            RATE_LIMITER
                .get_or_create(&RateLimiterLabels {})
                .set(self.size as i64);
        }

        // check the number of recent entries
        if value.len() >= self.entry_max_size {
            return false;
        }

        // enqueue
        value.push_back(Instant::now());
        self.size += 1;
        RATE_LIMITER
            .get_or_create(&RateLimiterLabels {})
            .set(self.size as i64);

        // cleanup if not recent (expect up to 100 full connections)
        if self.size > self.entry_max_size * 100 {
            self.cleanup();
        }

        true
    }

    /// Removes all expired timestamps from the entry map.
    fn cleanup(&mut self) {
        let mut expired = vec![];

        for (key, value) in self.entries.iter_mut() {
            while value
                .front()
                .is_some_and(|time| time.elapsed() > Duration::from_secs(10))
            {
                value.pop_front();
                self.size -= 1;
            }

            if value.is_empty() {
                expired.push(*key);
            }
        }

        for key in expired {
            self.entries.remove(&key);
        }

        RATE_LIMITER
            .get_or_create(&RateLimiterLabels {})
            .set(self.size as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn allow_initial() {
        tokio::time::pause();
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
    }

    #[tokio::test]
    async fn reject_many() {
        tokio::time::pause();
        let mut rate_limiter = RateLimiter::new(Duration::from_secs(10), 3);

        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(rate_limiter.enqueue(&0));
        assert!(!rate_limiter.enqueue(&0));

        tokio::time::advance(Duration::from_secs(9)).await;

        assert!(!rate_limiter.enqueue(&0));
    }

    #[tokio::test]
    async fn allow_after_duration() {
        tokio::time::pause();
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

    #[tokio::test]
    async fn allow_disjoint() {
        tokio::time::pause();
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
