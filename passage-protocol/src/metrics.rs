use opentelemetry::metrics::Meter;
use opentelemetry::{InstrumentationScope, global};
use std::sync::LazyLock;

static METER: LazyLock<Meter> = LazyLock::new(|| {
    let scope = InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();
    global::meter_with_scope(scope)
});

/// Generates exponential histogram buckets. Panics if `start` <= 0 or `factor` <= 1
///
/// # Arguments
/// * `start` - The value of the first bucket (must be > 0)
/// * `factor` - The exponential factor (must be > 1)
/// * `count` - The number of buckets to generate
fn exponential_buckets(start: f64, factor: f64, count: usize) -> Vec<f64> {
    assert!(start > 0.0, "start must be greater than 0");
    assert!(factor > 1.0, "factor must be greater than 1");

    (0..count).map(|i| start * factor.powi(i as i32)).collect()
}

pub(crate) mod request_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("request_duration")
            .with_description("The time a request took to complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    pub(crate) fn record(started: Instant, result: &'static str) {
        INSTRUMENT.record(
            started.elapsed().as_secs(),
            &[KeyValue::new("result", result)],
        )
    }
}

pub(crate) mod open_connections {
    use crate::metrics::METER;
    use opentelemetry::metrics::UpDownCounter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<UpDownCounter<i64>> = LazyLock::new(|| {
        METER
            .i64_up_down_counter("open_connections")
            .with_description("The number of currently open connections")
            .build()
    });

    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }

    pub(crate) fn dec() {
        INSTRUMENT.add(-1, &[])
    }
}

pub(crate) mod rate_limiter_size {
    use crate::metrics::METER;
    use opentelemetry::metrics::Gauge;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
        METER
            .u64_gauge("rate_limiter_size")
            .with_description("The number of entries in the rate limiter")
            .build()
    });

    pub(crate) fn set(amount: u64) {
        INSTRUMENT.record(amount, &[])
    }
}
