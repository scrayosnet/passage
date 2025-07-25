use crate::adapter::status::Protocol;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets, linear_buckets};
use prometheus_client::registry::Registry;
use std::sync::{Arc, LazyLock};

pub(crate) type HistogramFamily<T> = Family<T, Histogram, fn() -> Histogram>;

/// The application metrics registry.
pub(crate) static REGISTRY: LazyLock<Arc<Registry>> = LazyLock::new(build_registry);

pub(crate) static REQUESTS: LazyLock<Family<RequestsLabels, Counter>> =
    LazyLock::new(Family::<RequestsLabels, Counter>::default);

pub(crate) static RATE_LIMITER: LazyLock<Family<RateLimiterLabels, Gauge>> =
    LazyLock::new(Family::<RateLimiterLabels, Gauge>::default);

pub(crate) static OPEN_CONNECTIONS: LazyLock<Family<OpenConnectionsLabels, Gauge>> =
    LazyLock::new(Family::<OpenConnectionsLabels, Gauge>::default);

pub(crate) static CONNECTION_DURATION: LazyLock<HistogramFamily<ConnectionDurationLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<ConnectionDurationLabels>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10))
        })
    });

pub(crate) static RECEIVED_PACKETS: LazyLock<HistogramFamily<ReceivedPackets>> =
    LazyLock::new(|| {
        HistogramFamily::<ReceivedPackets>::new_with_constructor(|| {
            Histogram::new(linear_buckets(0.0, 512.0, 10))
        })
    });

pub(crate) static SENT_PACKETS: LazyLock<Family<SentPackets, Counter>> =
    LazyLock::new(Family::<SentPackets, Counter>::default);

pub(crate) static RESOURCEPACK_DURATION: LazyLock<HistogramFamily<ResourcePackDurationLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<ResourcePackDurationLabels>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10))
        })
    });

pub(crate) static TRANSFER_TARGETS: LazyLock<Family<TransferTargetsLabels, Counter>> =
    LazyLock::new(Family::<TransferTargetsLabels, Counter>::default);

pub(crate) static MOJANG_DURATION: LazyLock<HistogramFamily<MojangDurationLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<MojangDurationLabels>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10))
        })
    });

pub(crate) static CLIENT_LOCALES: LazyLock<Family<ClientLocaleLabels, Counter>> =
    LazyLock::new(Family::<ClientLocaleLabels, Counter>::default);

pub(crate) static CLIENT_VIEW_DISTANCE: LazyLock<HistogramFamily<ClientViewDistanceLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<ClientViewDistanceLabels>::new_with_constructor(|| {
            Histogram::new(linear_buckets(1.0, 8.0, 32))
        })
    });

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestsLabels {
    pub result: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RateLimiterLabels {}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct OpenConnectionsLabels {}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ConnectionDurationLabels {
    pub variant: &'static str,
    pub protocol_version: Protocol,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ReceivedPackets {}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SentPackets {}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ResourcePackDurationLabels {
    pub uuid: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TransferTargetsLabels {
    pub target: Option<String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct MojangDurationLabels {
    pub result: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ClientLocaleLabels {
    pub locale: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ClientViewDistanceLabels {}

pub(crate) struct Guard<F: FnOnce()> {
    func: Option<F>,
}

impl<F: FnOnce()> Guard<F> {
    pub(crate) fn on_drop(func: F) -> Self {
        Self { func: Some(func) }
    }
}

impl<F: FnOnce()> Drop for Guard<F> {
    fn drop(&mut self) {
        if let Some(func) = self.func.take() {
            func();
        }
    }
}

fn build_registry() -> Arc<Registry> {
    let mut registry = Registry::with_prefix("passage");

    registry.register("requests", "Number of requests", REQUESTS.clone());
    registry.register(
        "rate_limiter_size",
        "The number of entries in the rate limiter",
        RATE_LIMITER.clone(),
    );
    registry.register(
        "open_connections",
        "The number of currently open connections",
        OPEN_CONNECTIONS.clone(),
    );
    registry.register(
        "connection_duration_seconds",
        "Duration a (non-aborted) connection was processed for in seconds",
        CONNECTION_DURATION.clone(),
    );
    registry.register(
        "received_packets",
        "Number of received requests",
        RECEIVED_PACKETS.clone(),
    );
    registry.register(
        "sent_packets",
        "Number of sent requests",
        SENT_PACKETS.clone(),
    );
    registry.register(
        "resourcepack_duration_seconds",
        "Duration a resource pack took to load in seconds",
        RESOURCEPACK_DURATION.clone(),
    );
    registry.register(
        "transfer_targets",
        "Number of targets selected for transfer",
        TRANSFER_TARGETS.clone(),
    );
    registry.register(
        "mojang_duration_seconds",
        "Duration a mojang request took in seconds",
        MOJANG_DURATION.clone(),
    );
    registry.register(
        "client_locales",
        "Number of received client locales",
        CLIENT_LOCALES.clone(),
    );
    registry.register(
        "client_view_distance",
        "Configured client view distances",
        CLIENT_VIEW_DISTANCE.clone(),
    );

    Arc::new(registry)
}
