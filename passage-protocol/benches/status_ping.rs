//! End-to-end benchmark of the status ping path.
//!
//! Unlike the packet benchmarks, which measure serialisation in isolation, this drives complete
//! connections through a real listener over loopback TCP: accept, handshake, status request,
//! status response, ping, pong, close. The adapters are the built-in fixed ones and perform no
//! I/O, so what is measured is the cost Passage itself adds per connection.
//!
//! The numbers are only meaningful relative to each other on the same machine. The load generator
//! shares the CPU with the listener, so absolute throughput is a lower bound, and a real
//! deployment is dominated by whatever the configured adapters do.
//!
//! ```text
//! cargo bench -p passage-protocol --bench status_ping
//! ```

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use passage_protocol::config::Config;
use std::time::Duration;
use tokio::runtime::Runtime;

#[path = "../tests/support/mod.rs"]
mod support;

use support::Harness;

/// Concurrency levels to measure. One is the pure round-trip latency, the higher levels show how
/// far the listener scales before it saturates.
const CONCURRENCY: [usize; 4] = [1, 8, 64, 256];

fn status_ping(c: &mut Criterion) {
    let runtime = Runtime::new().expect("failed to build benchmark runtime");

    // A short connection timeout keeps the benchmark from piling up watchdog tasks for connections
    // that have long since completed, which would measure task bookkeeping instead of the protocol.
    let config = Config {
        connection_timeout: 1,
        ..Config::default()
    };
    let harness = runtime.block_on(Harness::start(config));
    let address = harness.address;

    let mut group = c.benchmark_group("status_ping");
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(50);

    for concurrency in CONCURRENCY {
        group.throughput(Throughput::Elements(concurrency as u64));
        group.bench_function(BenchmarkId::from_parameter(concurrency), |b| {
            b.to_async(&runtime).iter(|| async move {
                let pings = (0..concurrency).map(|_| support::status_ping(address));
                for result in futures::future::join_all(pings).await {
                    result.expect("status ping failed");
                }
            });
        });
    }

    group.finish();
    runtime.block_on(harness.shutdown());
}

criterion_group!(benches, status_ping);
criterion_main!(benches);
