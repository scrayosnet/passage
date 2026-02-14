use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use passage_protocol::rate_limiter;

fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("rw");

    group.bench_function(BenchmarkId::new("enqueue", 0), |b| {
        let mut limiter = rate_limiter::RateLimiter::new(tokio::time::Duration::from_secs(3), 3);
        b.iter(|| {
            for u in 0..10 {
                for _ in 0..100 {
                    limiter.enqueue(u);
                }
            }
        });
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
