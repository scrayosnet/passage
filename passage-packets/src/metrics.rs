use opentelemetry::metrics::Meter;
use opentelemetry::{InstrumentationScope, global};
use std::sync::LazyLock;

/// The meter used for all protocol metrics.
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
fn linear_buckets(start: f64, width: f64, count: usize) -> Vec<f64> {
    (0..count).map(|i| start + (i as f64 * width)).collect()
}

/// The metric `packet_size` tracks the size of packets encoded and decoded.
///
/// Attributes:
/// - `action`: `encoded` for encoded packets, `decoded` for decoded packets
pub(crate) mod packet_size {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("packet_size")
            .with_description("The size of packets encoded and decoded")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    /// Records the size of an encoded packet in bytes.
    pub(crate) fn record_encoded(size: u64) {
        INSTRUMENT.record(size, &[KeyValue::new("action", "encoded")])
    }

    /// Records the size of a decoded packet in bytes.
    pub(crate) fn record_decoded(size: u64) {
        INSTRUMENT.record(size, &[KeyValue::new("action", "decoded")])
    }
}

/// The metric `packet_bytes` tracks the total bytes of packets encoded and decoded.
///
/// Attributes:
/// - `action`: `encoded` for encoded packets, `decoded` for decoded packets
pub(crate) mod packet_bytes {
    use crate::metrics::METER;
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("packet_bytes")
            .with_description("The total bytes of packets encoded and decoded")
            .with_unit("bytes")
            .build()
    });

    /// Increments the total bytes encoded.
    pub(crate) fn add_encoded(size: u64) {
        INSTRUMENT.add(size, &[KeyValue::new("action", "encoded")])
    }

    /// Increments the total bytes decoded.
    pub(crate) fn add_decoded(size: u64) {
        INSTRUMENT.add(size, &[KeyValue::new("action", "decoded")])
    }
}
