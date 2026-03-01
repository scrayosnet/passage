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

/// The set of system metrics. The system metrics have to be refreshed manually or by using the [system::observe]
/// function which starts a background task that periodically updates the metrics.
pub mod system {
    use std::time::Duration;
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    use tokio::task::JoinHandle;
    use tokio::time::MissedTickBehavior;
    use tokio_util::sync::CancellationToken;
    use tracing::info;

    /// Starts a system observer that updates all system metrics periodically. This should only be
    /// called once by the application (not the library).
    pub fn observe(duration: Duration, stop: CancellationToken) -> JoinHandle<()> {
        tokio::spawn(async move {
            info!(interval = ?duration, "starting system observer");
            let mut interval = tokio::time::interval(duration);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let system_refresh = RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::nothing().with_ram())
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage());
            let mut system = System::new_with_specifics(system_refresh);
            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            loop {
                tokio::select! {
                    _ = stop.cancelled() => {
                        info!("stopped system observer");
                        return;
                    },
                    _ = interval.tick() => {
                        system.refresh_specifics(system_refresh);
                        cpu_usage::observe(system.global_cpu_usage());
                        total_memory::observe(system.total_memory());
                        free_memory::observe(system.free_memory());
                        available_memory::observe(system.available_memory());
                        used_memory::observe(system.used_memory());
                        total_swap::observe(system.total_swap());
                        free_swap::observe(system.free_swap());
                        used_swap::observe(system.used_swap());
                    }
                }
            }
        })
    }

    /// The metric `cpu_usage` tracks the global CPU usage percentage.
    pub(crate) mod cpu_usage {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<f64>> = LazyLock::new(|| {
            METER
                .f64_gauge("cpu_usage")
                .with_description("The global CPU usage percentage")
                .with_unit("percent")
                .build()
        });

        /// Sets cpu_usage.
        pub(crate) fn observe(amount: f32) {
            INSTRUMENT.record(amount as f64, &[])
        }
    }

    /// The metric `total_memory` tracks the total system memory in bytes.
    pub(crate) mod total_memory {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("total_memory")
                .with_description("The total system memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets total_memory.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `free_memory` tracks the free system memory in bytes.
    pub(crate) mod free_memory {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("free_memory")
                .with_description("The free system memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets free_memory.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `available_memory` tracks the available system memory in bytes.
    pub(crate) mod available_memory {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("available_memory")
                .with_description("The available system memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets available_memory.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `used_memory` tracks the memory used by the system in bytes.
    pub(crate) mod used_memory {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("used_memory")
                .with_description("The memory used by the system")
                .with_unit("bytes")
                .build()
        });

        /// Sets used_memory.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `total_swap` tracks the total swap memory in bytes.
    pub(crate) mod total_swap {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("total_swap")
                .with_description("The total swap memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets total_swap.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `free_swap` tracks the free swap memory in bytes.
    pub(crate) mod free_swap {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("free_swap")
                .with_description("The free swap memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets free_swap.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }

    /// The metric `used_swap` tracks the used swap memory in bytes.
    pub(crate) mod used_swap {
        use crate::metrics::METER;
        use opentelemetry::metrics::Gauge;
        use std::sync::LazyLock;

        static INSTRUMENT: LazyLock<Gauge<u64>> = LazyLock::new(|| {
            METER
                .u64_gauge("used_swap")
                .with_description("The used swap memory")
                .with_unit("bytes")
                .build()
        });

        /// Sets used_swap.
        pub(crate) fn observe(amount: u64) {
            INSTRUMENT.record(amount, &[])
        }
    }
}

/// The metric `rate_limiter_size` tracts the number of entries in the rate limiter. The rate limiter
/// should automatically flush itself to keep the number of entries small. This metric allows maintainers
/// to verify this behavior.
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

    /// Sets the rate limiter size.
    pub(crate) fn set(amount: u64) {
        INSTRUMENT.record(amount, &[])
    }
}

/// The metric `listener_requests` tracks the number of requests accepted by the listener independent
/// of the connection result. In contrary to the `connection_duration` metric, this metric
/// tracks any incoming request, not only those that are handled by the protocol.
///
/// Attributes:
/// - `decision`: `rejected` if the proxy protocol or rate limiter fails, otherwise `accepted`
pub(crate) mod requests {
    use crate::metrics::METER;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("listener_requests")
            .with_description("The number of incoming requests")
            .build()
    });

    /// Increments the counter as `accepted`.
    pub(crate) fn accept() {
        INSTRUMENT.add(1, &[opentelemetry::KeyValue::new("decision", "accepted")])
    }

    /// Increments the counter as `rejected`.
    pub(crate) fn reject() {
        INSTRUMENT.add(1, &[opentelemetry::KeyValue::new("decision", "rejected")])
    }
}

/// The metric `connection_duration` tracks the time in seconds a connection takes to complete. In
/// contrary to the `listener_requests` metric, this metric only tracks connections not rejected by
/// the rate limiter (or proxy protocol).
pub(crate) mod connection_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("connection_duration")
            .with_description("The time a connection took complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    /// Records the number of seconds elapsed by the given `started` instant.
    pub(crate) fn record(started: Instant) {
        INSTRUMENT.record(started.elapsed().as_secs(), &[])
    }
}

/// The metric `open_connections` tracks the number of currently open connections. It does not track
/// requests previously rejected by the rate limiter or proxy protocol.
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

    /// Increments the counter.
    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }

    /// Decrements the counter.
    pub(crate) fn dec() {
        INSTRUMENT.add(-1, &[])
    }
}

/// The metric `packet_size` tracks the size of packets received and sent.
///
/// Attributes:
/// - `bound`: `serverbound` for received packets, `clientbound` for sent packets
pub(crate) mod packet_size {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("packet_size")
            .with_description("The size of packets received and sent")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    /// Records the size of an incoming (received) packet in bytes.
    pub(crate) fn record_serverbound(size: u64) {
        INSTRUMENT.record(size, &[KeyValue::new("bound", "serverbound")])
    }

    /// Records the size of an outgoing (sent) packet in bytes.
    pub(crate) fn record_clientbound(size: u64) {
        INSTRUMENT.record(size, &[KeyValue::new("bound", "clientbound")])
    }
}

/// The metric `client_locales` tracks the locals sent by the clients. Technically, a client may send
/// this information multiple times resulting in multiple increments.
///
/// Attributes:
/// - `locale`: The locale sent by the client (technically any value)
pub(crate) mod client_locales {
    use crate::metrics::METER;
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("client_locales")
            .with_description("The number of clients using some locale")
            .build()
    });

    /// Increments the counter for the given locale.
    pub(crate) fn inc(locale: String) {
        INSTRUMENT.add(1, &[KeyValue::new("locale", locale)])
    }
}

/// The metric `client_view_distances` tracks the view distance sent by the clients. Technically, a client may send
/// this information multiple times resulting in multiple increments.
pub(crate) mod client_view_distances {
    use crate::metrics::{METER, linear_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("client_view_distances")
            .with_description("The view distance of clients")
            .with_unit("bytes")
            .with_boundaries(linear_buckets(0.0, 512.0, 10))
            .build()
    });

    /// Records the view distance.
    pub(crate) fn record(distance: u64) {
        INSTRUMENT.record(distance, &[])
    }
}

/// The metric `status_connections` tracks the number of connections making a status request.
pub(crate) mod status_connections {
    use crate::metrics::METER;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("status_connections")
            .with_description("The number of connections making status requests")
            .build()
    });

    /// Increments the counter.
    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }
}

/// The metric `transfer_connections` tracks the number of connections making a login or transfer request.
pub(crate) mod transfer_connections {
    use crate::metrics::METER;
    use opentelemetry::metrics::Counter;
    use std::sync::LazyLock;

    static INSTRUMENT: LazyLock<Counter<u64>> = LazyLock::new(|| {
        METER
            .u64_counter("transfer_connections")
            .with_description("The number of connections making status and transfer requests")
            .build()
    });

    /// Increments the counter.
    pub(crate) fn inc() {
        INSTRUMENT.add(1, &[])
    }
}

/// The metric `status_duration` tracks the time in seconds the status adapter takes to complete.
pub(crate) mod status_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("status_duration")
            .with_description("The time the status adapter took to complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    /// Records the number of seconds elapsed by the given `started` instant.
    pub(crate) fn record(started: Instant) {
        INSTRUMENT.record(started.elapsed().as_secs(), &[])
    }
}

/// The metric `selection_duration` tracks the time in seconds a target selection takes to complete.
pub(crate) mod selection_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("selection_duration")
            .with_description("The time a target selection took to complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    /// Records the number of seconds elapsed by the given `started` instant.
    pub(crate) fn record(started: Instant) {
        INSTRUMENT.record(started.elapsed().as_secs(), &[])
    }
}

/// The metric `authentication_duration` tracks the time in seconds the auth adapter takes to complete.
pub(crate) mod authentication_duration {
    use crate::metrics::{METER, exponential_buckets};
    use opentelemetry::metrics::Histogram;
    use std::sync::LazyLock;
    use tokio::time::Instant;

    static INSTRUMENT: LazyLock<Histogram<u64>> = LazyLock::new(|| {
        METER
            .u64_histogram("authentication_request_duration")
            .with_description("The time the authentication adapter took to complete")
            .with_unit("seconds")
            .with_boundaries(exponential_buckets(0.1, 2.0, 10))
            .build()
    });

    /// Records the number of seconds elapsed by the given `started` instant.
    pub(crate) fn record(started: Instant) {
        INSTRUMENT.record(started.elapsed().as_secs(), &[])
    }
}
