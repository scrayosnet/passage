//! Everything that has to be standing before the first connection is accepted: the subscriber that
//! logs, the providers that export, and the error reporter.
//!
//! All of it is optional and driven by [`Config`]. An endpoint that is not configured is not
//! exported to, and the process still logs to stdout -- so a deployment without a collector behaves
//! like one with the collector switched off, not like one that fails to start.

use crate::config::Config;
use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{
    LogExporter, MetricExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions::resource::SERVICE_NAMESPACE;
use opentelemetry_semantic_conventions::{
    SCHEMA_URL,
    attribute::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_INSTANCE_ID, SERVICE_VERSION},
};
use std::collections::HashMap;
use std::sync::LazyLock;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;
use uuid::Uuid;

/// The instrumentation scope reported for everything this process exports.
const SERVICE_NAME: &str = "passage";

/// The namespace every Passage deployment is reported under.
const SERVICE_NAMESPACE_VALUE: &str = "scrayosnet";

/// The instance id of this service. It is generated once for each deployment.
const INSTANCE_ID: LazyLock<Uuid> = LazyLock::new(|| Uuid::new_v4());

/// Holds everything that has to outlive the server, and flushes it when it does not.
///
/// The exporters batch, so a process that ends without shutting them down loses whatever had not
/// been sent yet -- which is exactly the tail that explains why it ended. Dropping this guard is
/// what flushes them, so it has to be kept alive for as long as anything is worth recording.
#[must_use = "telemetry is shut down when this guard is dropped"]
pub struct Telemetry {
    meter_provider: Option<SdkMeterProvider>,
    tracer_provider: Option<SdkTracerProvider>,
    logger_provider: Option<SdkLoggerProvider>,

    /// Kept only to be dropped with the rest: sentry stops reporting once its guard goes away.
    #[cfg(feature = "sentry")]
    _sentry: Option<sentry::ClientInitGuard>,
}

impl Drop for Telemetry {
    fn drop(&mut self) {
        if let Some(Err(err)) = self
            .meter_provider
            .take()
            .map(|provider| provider.shutdown())
        {
            warn!(err = %err, "failed to close the meter provider");
        }
        if let Some(Err(err)) = self
            .tracer_provider
            .take()
            .map(|provider| provider.shutdown())
        {
            warn!(err = %err, "failed to close the tracer provider");
        }
        if let Some(Err(err)) = self
            .logger_provider
            .take()
            .map(|provider| provider.shutdown())
        {
            warn!(err = %err, "failed to close the logger provider");
        }
    }
}

/// The resource every signal is tagged with: what this is, which version, and where it runs.
fn resource(environment: &str) -> Resource {
    Resource::builder()
        .with_service_name(SERVICE_NAME)
        .with_schema_url(
            [
                KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                KeyValue::new(SERVICE_NAMESPACE, SERVICE_NAMESPACE_VALUE),
                KeyValue::new(SERVICE_INSTANCE_ID, INSTANCE_ID.to_string()),
                KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, environment.to_owned()),
            ],
            SCHEMA_URL,
        )
        .build()
}

/// The `authorization` header an OTLP endpoint is addressed with.
fn headers(token: &str) -> HashMap<String, String> {
    HashMap::from_iter([("authorization".to_owned(), format!("Basic {token}"))])
}

/// Installs the global subscriber, the OpenTelemetry providers and the error reporter.
///
/// This sets process-global state and so may only be called once; a second call leaves the first
/// subscriber in place. The returned guard flushes everything when it is dropped, which is why it
/// has to be held for the lifetime of the process.
///
/// # Errors
///
/// Returns an error if a configured OTLP exporter cannot be built, which is a malformed endpoint
/// rather than an unreachable one -- an endpoint that is merely down is retried by the exporter.
pub fn init_tracing(config: &Config) -> Result<Telemetry, Box<dyn std::error::Error>> {
    // Sentry has to be initialized before the subscriber that feeds it.
    #[cfg(feature = "sentry")]
    let sentry_guard = config.sentry.as_ref().map(|sentry_config| {
        sentry::init((
            sentry_config.address.clone(),
            sentry::ClientOptions {
                debug: sentry_config.debug,
                release: sentry::release_name!(),
                environment: Some(std::borrow::Cow::Owned(sentry_config.environment.clone())),
                ..sentry::ClientOptions::default()
            },
        ))
    });

    // Metrics: exported periodically, and registered globally so that `metrics` can record into it.
    let meter_provider = match &config.otel.metrics {
        None => None,
        Some(endpoint) => {
            let exporter = MetricExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&endpoint.address)
                .with_headers(headers(&endpoint.token))
                .build()?;
            let provider = SdkMeterProvider::builder()
                .with_periodic_exporter(exporter)
                .with_resource(resource(&config.otel.environment))
                .build();
            global::set_meter_provider(provider.clone());
            Some(provider)
        }
    };

    // Spans: batched, and with the W3C propagator installed so that the trace a session cookie
    // carries is linked from the connection that presents it. Without an endpoint there is no
    // propagator either, and the link is simply not made -- see `on_login_encryption_response`.
    let tracer_provider = match &config.otel.traces {
        None => None,
        Some(endpoint) => {
            let exporter = SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&endpoint.address)
                .with_headers(headers(&endpoint.token))
                .build()?;
            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource(&config.otel.environment))
                .build();
            global::set_tracer_provider(provider.clone());
            global::set_text_map_propagator(TraceContextPropagator::new());
            Some(provider)
        }
    };

    // Logs: the same events that go to stdout, bridged to the collector.
    let logger_provider = match &config.otel.logs {
        None => None,
        Some(endpoint) => {
            let exporter = LogExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&endpoint.address)
                .with_headers(headers(&endpoint.token))
                .build()?;
            Some(
                SdkLoggerProvider::builder()
                    .with_batch_exporter(exporter)
                    .with_resource(resource(&config.otel.environment))
                    .build(),
            )
        }
    };

    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(tracing_subscriber::fmt::layer().compact())
        // Each of these is `None` when its endpoint is not configured, and a `None` layer is one
        // that is not installed at all.
        .with(
            meter_provider
                .as_ref()
                .map(|provider| MetricsLayer::new(provider.clone())),
        )
        .with(
            tracer_provider
                .as_ref()
                .map(|provider| OpenTelemetryLayer::new(provider.tracer(SERVICE_NAME))),
        )
        .with(
            logger_provider
                .as_ref()
                .map(OpenTelemetryTracingBridge::new),
        );

    #[cfg(feature = "sentry")]
    let subscriber = subscriber.with(sentry_tracing::layer());

    subscriber.init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        name = SERVICE_NAME,
        environment = config.otel.environment,
        metrics = meter_provider.is_some(),
        traces = tracer_provider.is_some(),
        logs = logger_provider.is_some(),
        "initialized telemetry"
    );

    Ok(Telemetry {
        meter_provider,
        tracer_provider,
        logger_provider,
        #[cfg(feature = "sentry")]
        _sentry: sentry_guard,
    })
}
