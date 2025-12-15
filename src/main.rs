use opentelemetry::trace::TracerProvider;
use opentelemetry::{KeyValue, global};
use opentelemetry_otlp::{
    MetricExporter, Protocol, SpanExporter, WithExportConfig, WithHttpConfig,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::MeterProviderBuilder;
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_semantic_conventions::resource::SERVICE_NAMESPACE;
use opentelemetry_semantic_conventions::{
    SCHEMA_URL,
    attribute::{DEPLOYMENT_ENVIRONMENT_NAME, SERVICE_VERSION},
};
use passage::config::{Config, OpenTelemetry};
use std::borrow::Cow::Owned;
use std::collections::HashMap;
use std::env;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource(config: &OpenTelemetry) -> Resource {
    Resource::builder()
        .with_service_name(env!("CARGO_PKG_NAME"))
        .with_schema_url(
            [
                KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                KeyValue::new(SERVICE_NAMESPACE, env!("CARGO_PKG_NAME")),
                KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, config.environment.to_string()),
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

    // build future to execute
    let run = async {
        // initialize opentelemetry meter (metrics)
        let metrics_headers = HashMap::from_iter([(
            "authorization".to_string(),
            format!("Basic {}", config.otel.metrics_token),
        )]);
        let meter_exporter = MetricExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(&config.otel.metrics_endpoint)
            .with_headers(metrics_headers.clone())
            .build()?;
        let meter_provider = MeterProviderBuilder::default()
            .with_periodic_exporter(meter_exporter)
            .with_resource(resource(&config.otel))
            .build();
        global::set_meter_provider(meter_provider.clone());

        // initialize opentelemetry tracer (spans)
        let traces_headers = HashMap::from_iter([(
            "authorization".to_string(),
            format!("Basic {}", config.otel.traces_token),
        )]);
        let tracer_exporter = SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(&config.otel.traces_endpoint)
            .with_headers(traces_headers.clone())
            .build()?;
        let tracer_provider = SdkTracerProvider::builder()
            .with_batch_exporter(tracer_exporter)
            .with_resource(resource(&config.otel))
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
        info!(
            version = env!("CARGO_PKG_VERSION"),
            name = env!("CARGO_PKG_NAME"),
            "starting passage"
        );

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
        let result = passage::start(config).await;

        // shutdown opentelemetry
        if let Err(err) = meter_provider.shutdown() {
            warn!(err = %err, "Error while closing meter provider");
        }
        if let Err(err) = tracer_provider.shutdown() {
            warn!(err = %err, "Error while closing trace provider");
        }

        result
    };

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run)
}
