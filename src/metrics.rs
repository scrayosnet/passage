use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::registry::Registry;
use std::sync::{Arc, LazyLock};

/// The application metrics registry.
pub(crate) static REGISTRY: LazyLock<Arc<Registry>> = LazyLock::new(build_registry);

pub(crate) static REQUESTS: LazyLock<Family<RequestLabels, Counter>> =
    LazyLock::new(Family::<RequestLabels, Counter>::default);

pub(crate) static RATE_LIMITED: LazyLock<Family<RateLimitedLabels, Counter>> =
    LazyLock::new(Family::<RateLimitedLabels, Counter>::default);

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RequestLabels {}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RateLimitedLabels {}

fn build_registry() -> Arc<Registry> {
    let mut registry = Registry::with_prefix("passage");

    registry.register("requests", "Number of requests", REQUESTS.clone());
    registry.register(
        "rate_limited_requests",
        "Number of rate limited requests",
        RATE_LIMITED.clone(),
    );

    Arc::new(registry)
}
