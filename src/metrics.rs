use crate::adapter::status::Protocol;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;
use std::sync::{Arc, LazyLock};

pub(crate) type HistogramFamily<T> = Family<T, Histogram, fn() -> Histogram>;

/// The application metrics registry.
pub(crate) static REGISTRY: LazyLock<Arc<Registry>> = LazyLock::new(build_registry);

pub(crate) static REQUESTS: LazyLock<Family<RequestsLabels, Counter>> =
    LazyLock::new(Family::<RequestsLabels, Counter>::default);

pub(crate) static REQUEST_DURATION: LazyLock<HistogramFamily<RequestDurationLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<RequestDurationLabels>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10))
        })
    });

pub(crate) static RECEIVED_PACKETS: LazyLock<Family<ReceivedPackets, Counter>> =
    LazyLock::new(Family::<ReceivedPackets, Counter>::default);

pub(crate) static SENT_PACKETS: LazyLock<Family<SentPackets, Counter>> =
    LazyLock::new(Family::<SentPackets, Counter>::default);

pub(crate) static RESOURCEPACK_DURATION: LazyLock<HistogramFamily<ResourcePackDurationLabels>> =
    LazyLock::new(|| {
        HistogramFamily::<ResourcePackDurationLabels>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10))
        })
    });

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestsLabels {
    pub result: &'static str,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestDurationLabels {
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
        "request_duration_seconds",
        "Duration a request was processed for in seconds",
        REQUEST_DURATION.clone(),
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
    // TODO transfer target
    // TODO mojang success

    Arc::new(registry)
}
