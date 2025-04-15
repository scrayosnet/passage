use crate::adapter::resourcepack::ResourcepackSupplier;
use crate::adapter::status::StatusSupplier;
use crate::adapter::target_selection::TargetSelector;
use crate::connection::Connection;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::timeout;
use tracing::{info, warn};

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

        // check whether client is already registered, otherwise add
        let Some(value) = self.entries.get_mut(key) else {
            self.entries
                .insert(*key, VecDeque::from_iter([Instant::now()]));
            self.size += 1;
            return true;
        };

        // clear non-recent entries
        while let Some(front) = value.front() {
            if front.elapsed() < self.duration {
                break;
            }
            value.pop_front();
            self.size -= 1;
        }

        // check number of recent entries
        if value.len() > self.entry_max_size {
            return false;
        }

        // enqueue
        value.push_back(Instant::now());
        self.size += 1;

        // cleanup if not recent (expect up to 100 full connections)
        if self.size > self.entry_max_size * 100 {
            self.cleanup()
        }

        true
    }

    /// Removes all expired timestamps from the entries map.
    fn cleanup(&mut self) {
        let mut expired = vec![];

        for (key, value) in self.entries.iter_mut() {
            while value
                .front()
                .map_or(false, |time| time.elapsed() > Duration::from_secs(10))
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
    }
}

pub async fn serve(
    listener: TcpListener,
    status_supplier: Arc<dyn StatusSupplier>,
    target_selector: Arc<dyn TargetSelector>,
    resourcepack_supplier: Arc<dyn ResourcepackSupplier>,
) -> Result<(), Box<dyn std::error::Error>> {
    // setup rate limiting
    let rate_limiter_enabled = true;
    let mut rate_limiter = RateLimiter::new(Duration::from_secs(60), 60);

    loop {
        // accept the next incoming connection
        let (mut stream, addr) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = tokio::signal::ctrl_c() => {
                info!("received connection ctrl_c signal");
                return Ok(());
            },
        };

        // check rate limiter
        if rate_limiter_enabled && !rate_limiter.enqueue(&addr.ip()) {
            info!(addr = ?addr, "rate limited client");
            if let Err(e) = stream.shutdown().await {
                info!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failed to close a client connection"
                );
            }
            continue;
        }

        // clone values to be moved
        let status_supplier = Arc::clone(&status_supplier);
        let target_selector = Arc::clone(&target_selector);
        let resourcepack_supplier = Arc::clone(&resourcepack_supplier);

        tokio::spawn(timeout(Duration::from_secs(5 * 60), async move {
            // build connection wrapper for stream
            let mut con = Connection::new(
                &mut stream,
                addr,
                Arc::clone(&status_supplier),
                Arc::clone(&target_selector),
                Arc::clone(&resourcepack_supplier),
            );

            // handle the client connection
            if let Err(e) = con.listen().await {
                warn!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failure communicating with a client"
                );
            }

            // flush connection and shutdown
            if let Err(e) = stream.shutdown().await {
                info!(
                    cause = e.to_string(),
                    addr = &addr.to_string(),
                    "failed to close a client connection"
                );
            }

            info!(addr = &addr.to_string(), "closed connection with a client");
        }));
    }
}
