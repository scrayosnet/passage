use opentelemetry::metrics::Meter;
use opentelemetry::{InstrumentationScope, global};
use std::sync::LazyLock;

static METER: LazyLock<Meter> = LazyLock::new(|| {
    let scope = InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();
    global::meter_with_scope(scope)
});

/// Generates linear histogram buckets.
///
/// # Arguments
/// * `start` - The value of the first bucket
/// * `width` - The width of each bucket
/// * `count` - The number of buckets to generate
///
/// # Example
/// ```
/// let buckets = linear_buckets(0.0, 10.0, 5);
/// // Returns: [0.0, 10.0, 20.0, 30.0, 40.0]
/// ```
fn linear_buckets(start: f64, width: f64, count: usize) -> Vec<f64> {
    (0..count).map(|i| start + (i as f64 * width)).collect()
}

/// Generates exponential histogram buckets. Panics if `start` <= 0 or `factor` <= 1
///
/// # Arguments
/// * `start` - The value of the first bucket (must be > 0)
/// * `factor` - The exponential factor (must be > 1)
/// * `count` - The number of buckets to generate
///
/// # Example
/// ```
/// let buckets = exponential_buckets(1.0, 2.0, 5);
/// // Returns: [1.0, 2.0, 4.0, 8.0, 16.0]
/// ```
fn exponential_buckets(start: f64, factor: f64, count: usize) -> Vec<f64> {
    assert!(start > 0.0, "start must be greater than 0");
    assert!(factor > 1.0, "factor must be greater than 1");

    (0..count).map(|i| start * factor.powi(i as i32)).collect()
}

pub(crate) mod requests {
    use crate::metrics::METER;
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("requests")
            .with_description("Number of requests")
            .build()
    });

    // TODO should this have a label at all?
    pub(crate) fn inc(result: &'static str) {
        INSTRUMENT.add(1, &[KeyValue::new("result", result)])
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

pub(crate) mod rate_limited {
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

// TODO combine with other metric?
pub(crate) mod incoming_packets {
    use crate::metrics::METER;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("incoming_packets")
            .with_description("The number of incoming packets")
            .build()
    });

    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }
}

// TODO combine with other metric?
pub(crate) mod incoming_packet_size {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("incoming_packet_size")
            .with_description("The size of incoming packets")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    pub(crate) fn record(size: u64) {
        INSTRUMENT.record(size, &[])
    }
}

// TODO combine with other metric?
pub(crate) mod outgoing_packets {
    use crate::metrics::METER;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("outgoing_packets")
            .with_description("The number of outgoing packets")
            .build()
    });

    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }
}

// TODO combine with other metric?
pub(crate) mod outgoing_packet_size {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("outgoing_packet_size")
            .with_description("The size of outgoing packets")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    pub(crate) fn record(amount: u64) {
        INSTRUMENT.record(amount, &[])
    }
}

pub(crate) mod client_locale {
    use crate::metrics::METER;
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("client_locale")
            .with_description("The number of clients using some locale")
            .build()
    });

    pub(crate) fn inc(locale: String) {
        INSTRUMENT.add(1, &[KeyValue::new("locale", locale)])
    }
}

pub(crate) mod client_view_distance {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("client_view_distance")
            .with_description("The view distance of clients")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    pub(crate) fn record(distance: u64) {
        INSTRUMENT.record(distance, &[])
    }
}
