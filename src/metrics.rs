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

/// The set of system metrics. The system metrics have to be refreshed manually or by using the [system::observe]
/// function which starts a background task that periodically updates the metrics.
pub mod system {
    use std::time::Duration;
    use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
    use tokio::task::JoinHandle;
    use tokio::time::MissedTickBehavior;
    use tokio_util::sync::CancellationToken;
    use tracing::{info, warn};

    /// The default interval in seconds at which the system metrics are updated.
    pub const DEFAULT_OBSERVE_INTERVAL: u64 = 20;

    /// The system observer wraps a periodic task that updates the system metrics.
    pub struct Observer {
        cancellation_token: CancellationToken,
        handle: JoinHandle<()>,
    }

    impl Observer {
        /// Starts a system observer that updates all system metrics periodically. This should only be
        /// called once by the application (not the library).
        pub fn new(duration: Duration) -> Self {
            let cancellation_token = CancellationToken::new();
            let stop = cancellation_token.clone();
            let handle = tokio::spawn(async move {
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
            });
            Self {
                handle,
                cancellation_token,
            }
        }

        /// Stops the system observer.
        pub async fn shutdown(self) {
            self.cancellation_token.cancel();
            if let Err(err) = self.handle.await {
                warn!(err = ?err, "Error while shutting down system observer")
            }
        }
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
