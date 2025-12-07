use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{MeterProviderBuilder, PeriodicReader};
use opentelemetry_sdk::trace::{RandomIdGenerator, Sampler, SdkTracerProvider};
use opentelemetry_semantic_conventions::{
    SCHEMA_URL,
    attribute::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_VERSION},
};
use passage::config::Config;
use std::borrow::Cow::Owned;
use std::env;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource(environment: String) -> Resource {
    Resource::builder()
        .with_service_name(env!("CARGO_PKG_NAME"))
        .with_schema_url(
            [
                KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, environment),
            ],
            SCHEMA_URL,
        )
        .build()
}

/// Initializes the application and invokes passage.
///
/// This initializes the logging, aggregates configuration and starts the multithreaded tokio runtime. This is only a
/// thin-wrapper around the passage crate that supplies the necessary settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse the arguments and configuration
    let config = Config::new()?;

    // initialize sentry
    #[cfg(feature = "sentry")]
    let sentry_instance = sentry::init((
        config
            .sentry
            .enabled
            .then_some(config.sentry.address.clone()),
        sentry::ClientOptions {
            debug: config.sentry.debug,
            release: sentry::release_name!(),
            environment: Some(Owned(config.sentry.environment.clone())),
            ..sentry::ClientOptions::default()
        },
    ));

    // initialize opentelemetry
    let meter_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_http()
        .with_endpoint("http://localhost:5001")
        .with_temporality(opentelemetry_sdk::metrics::Temporality::default())
        .build()?;
    let meter_reader = PeriodicReader::builder(meter_exporter)
        .with_interval(std::time::Duration::from_secs(30))
        .build();
    let meter_provider = MeterProviderBuilder::default()
        .with_resource(resource(config.sentry.environment.clone()))
        .with_reader(meter_reader)
        .build();
    global::set_meter_provider(meter_provider.clone());

    let tracer_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint("http://localhost:5001")
        .build()?;
    let tracer_provider = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(
            1.0,
        ))))
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource(config.sentry.environment.clone()))
        .with_batch_exporter(tracer_exporter)
        .build();
    global::set_tracer_provider(tracer_provider.clone());

    // initialize logging
    let subscriber = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().compact().with_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            ),
        )
        .with(MetricsLayer::new(meter_provider.clone()))
        .with(OpenTelemetryLayer::new(tracer_provider.tracer("passage")));

    #[cfg(feature = "sentry")]
    let subscriber = subscriber.with(sentry_tracing::layer());

    subscriber.init();
    info!(version = env!("CARGO_PKG_VERSION"), "starting passage");

    #[cfg(feature = "sentry")]
    if sentry_instance.is_enabled() {
        info!("sentry is enabled");
    }

    if config.auth_secret.is_some() {
        info!("auth cookie is enabled");
    }

    let locale = config.localization.localize_default("locale", &[]);
    info!(locale = locale, "using localization");

    // run passage blocking
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async { passage::start(config).await });

    // shutdown opentelemetry
    if let Err(err) = meter_provider.shutdown() {
        warn!(err = %err, "Error while closing meter provider");
    }
    if let Err(err) = tracer_provider.shutdown() {
        warn!(err = %err, "Error while closing trace provider");
    }
    result
}
