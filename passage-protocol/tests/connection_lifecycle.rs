//! Tests for the resources a connection leaves behind after it has completed.
//!
//! Passage is designed to hold no state about a player once the connection is gone. These tests
//! verify that claim from the outside: they drive real status pings through a real listener and
//! then check what the runtime is still holding on to.
//!
//! The tests deliberately count tokio tasks instead of bytes. Resident memory is a poor signal
//! because the allocator keeps freed pages mapped, so RSS never falls back even when everything has
//! been released. The task count is exact and drops the moment a task actually finishes.
//!
//! Run the reporting profile (ignored by default, takes a while) with:
//!
//! ```text
//! cargo test --release -p passage-protocol --test connection_lifecycle -- --ignored --nocapture
//! ```

mod support;

use passage_protocol::config::Config;
use std::time::{Duration, Instant};
use support::{Harness, alive_tasks, connect_only, resident_kb, status_ping};

/// Number of pings used by the lifecycle assertions. Large enough that lingering tasks are
/// unmistakable, small enough to stay fast in CI.
const PINGS: usize = 200;

/// Polls `alive_tasks` until it drops to `target` or the deadline expires, and returns the last
/// observed value.
async fn drain_to(target: usize, timeout: Duration) -> usize {
    let deadline = Instant::now() + timeout;
    loop {
        let alive = alive_tasks();
        if alive <= target || Instant::now() >= deadline {
            return alive;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// What a load wave does per connection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// A complete status ping: handshake, status request, response, ping, pong.
    Status,
    /// Open a TCP connection and close it again without speaking any protocol. Isolates what the
    /// accept path costs from what packet handling costs.
    Connect,
}

/// Runs `count` connections with at most `concurrency` in flight.
///
/// The load is generated from futures rather than spawned tasks, so the caller's task count
/// measures the listener and not the load generator.
async fn wave(harness: &Harness, mode: Mode, count: usize, concurrency: usize) {
    let address = harness.address;
    for chunk in 0..count.div_ceil(concurrency) {
        let batch = (count - chunk * concurrency).min(concurrency);
        match mode {
            Mode::Status => {
                let futures = (0..batch).map(|_| status_ping(address));
                for result in futures::future::join_all(futures).await {
                    result.expect("status ping failed");
                }
            }
            Mode::Connect => {
                let futures = (0..batch).map(|_| connect_only(address));
                for result in futures::future::join_all(futures).await {
                    result.expect("connect failed");
                }
            }
        }
    }
}

/// Convenience wrapper for a wave of complete status pings.
async fn ping_wave(harness: &Harness, count: usize, concurrency: usize) {
    wave(harness, Mode::Status, count, concurrency).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn status_ping_round_trip_succeeds() {
    let harness = Harness::start_default().await;

    let body = harness.status_ping().await.expect("status ping failed");
    let status: serde_json::Value = serde_json::from_str(&body).expect("status is not valid JSON");

    assert_eq!(status["version"]["name"], "Passage Harness");
    assert_eq!(status["players"]["max"], 100);

    harness.shutdown().await;
}

/// A connection that has run to completion must not keep any task alive.
///
/// Every accepted connection spawns two tasks: the connection itself and a watchdog that cancels it
/// once `connection_timeout` elapses. If the watchdog is not stopped when the connection finishes
/// on its own, it stays parked on its timer for the full timeout. At a few thousand pings per
/// second and the default timeout of two minutes that is millions of live tasks, and the resident
/// memory to match.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_connections_release_their_tasks() {
    let harness = Harness::start_default().await;
    let baseline = alive_tasks();

    ping_wave(&harness, PINGS, 16).await;

    // Give the connection tasks a generous moment to finish, but far less than the default
    // connection timeout of 120 seconds.
    let alive = drain_to(baseline, Duration::from_secs(5)).await;

    assert!(
        alive <= baseline,
        "{PINGS} completed status pings left {} task(s) alive above the baseline of {baseline}; \
         connections that finished on their own must not keep tasks parked",
        alive.saturating_sub(baseline),
    );

    harness.shutdown().await;
}

/// Whatever a completed connection leaves behind must at least be released once the connection
/// timeout has elapsed, so the footprint is bounded by `rate * connection_timeout` rather than
/// growing without end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lingering_tasks_are_bounded_by_the_connection_timeout() {
    let config = Config {
        connection_timeout: 1,
        ..Config::default()
    };
    let harness = Harness::start(config).await;
    let baseline = alive_tasks();

    ping_wave(&harness, PINGS, 16).await;

    // Report, but do not assert, how much is still parked right after the wave: that number is what
    // `completed_connections_release_their_tasks` covers.
    println!(
        "directly after {PINGS} pings: {} task(s) above baseline",
        alive_tasks().saturating_sub(baseline),
    );

    let alive = drain_to(baseline, Duration::from_secs(15)).await;

    assert!(
        alive <= baseline,
        "{} task(s) were still alive 15s after a wave of {PINGS} pings with a 1s connection \
         timeout; leftovers must be released once the timeout elapses",
        alive.saturating_sub(baseline),
    );

    harness.shutdown().await;
}

/// Prints how tasks and resident memory develop over repeated load waves.
///
/// This is a reporting harness, not an assertion. It exists to make the growth reproducible and to
/// show whether it plateaus (bounded by the connection timeout) or accumulates without end.
///
/// Tunable through the environment:
/// `PROFILE_WAVES`, `PROFILE_PINGS`, `PROFILE_CONCURRENCY`, `PROFILE_TIMEOUT` and `PROFILE_MODE`
/// (`status` for full pings, `connect` for bare TCP connections).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "load profile, run explicitly with --ignored --nocapture"]
async fn memory_growth_profile() {
    fn env(key: &str, fallback: u64) -> u64 {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(fallback)
    }

    let waves = env("PROFILE_WAVES", 4) as usize;
    let pings = env("PROFILE_PINGS", 20_000) as usize;
    let concurrency = env("PROFILE_CONCURRENCY", 64) as usize;
    let timeout = env("PROFILE_TIMEOUT", 120);
    let mode = match std::env::var("PROFILE_MODE").as_deref() {
        Ok("connect") => Mode::Connect,
        Ok("status") | Err(_) => Mode::Status,
        Ok(other) => panic!("PROFILE_MODE must be `status` or `connect`, got `{other}`"),
    };
    let mode_label = if mode == Mode::Status {
        "status pings"
    } else {
        "bare connects"
    };

    let config = Config {
        connection_timeout: timeout,
        ..Config::default()
    };
    let harness = Harness::start(config).await;

    let report = |label: &str| {
        println!(
            "{label:<22} rss={:>8} kB   alive tasks={:>9}",
            resident_kb().map_or("n/a".to_owned(), |kb| kb.to_string()),
            alive_tasks(),
        );
    };

    println!(
        "\nprofile: {waves} wave(s) x {pings} {mode_label}, concurrency {concurrency}, \
         connection_timeout {timeout}s\n"
    );
    report("idle");

    for index in 1..=waves {
        let started = Instant::now();
        wave(&harness, mode, pings, concurrency).await;
        let elapsed = started.elapsed();
        report(&format!(
            "after wave {index} ({:.0}/s)",
            pings as f64 / elapsed.as_secs_f64()
        ));
    }

    // Wait out the connection timeout: anything tied to it must be gone afterwards. Resident memory
    // usually stays put because the allocator keeps the pages, so watch the task count instead.
    let cooldown = Duration::from_secs(timeout + 5);
    println!(
        "\nwaiting {}s for the connection timeout ...",
        cooldown.as_secs()
    );
    tokio::time::sleep(cooldown).await;
    report("after cooldown");

    harness.shutdown().await;
}
