use opentelemetry::metrics::Meter;
use opentelemetry::{InstrumentationScope, global};
use std::sync::LazyLock;

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

/// The meter used for all protocol metrics.
static METER: LazyLock<Meter> = LazyLock::new(|| {
    let scope = InstrumentationScope::builder(env!("CARGO_PKG_NAME"))
        .with_version(env!("CARGO_PKG_VERSION"))
        .build();
    global::meter_with_scope(scope)
});

/// The metric `adapter_duration` tracks the time in seconds an adapter takes to complete.
pub mod adapter_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("adapter_duration")
            .with_description("The time an adapter took to complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    /// Records the number of seconds elapsed by the given `started` instant. The `adapter_type` should
    /// be the name of the adapter, including the adapter type, e.g. `minecraft_auth_adapter`.
    pub fn record(adapter_type: &'static str, started: Instant) {
        INSTRUMENT.record(
            started.elapsed().as_secs(),
            &[KeyValue::new("adapter", adapter_type)],
        )
    }
}
