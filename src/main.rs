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
use passage::config::Config;
use std::borrow::Cow::Owned;
use std::collections::HashMap;
use std::env;
use tracing::level_filters::LevelFilter;
use tracing::{info, warn};
use tracing_opentelemetry::{MetricsLayer, OpenTelemetryLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

// Create a Resource that captures information about the entity for which telemetry is recorded.
fn resource(environment: &str) -> Resource {
    Resource::builder()
        .with_service_name(env!("CARGO_PKG_NAME"))
        .with_schema_url(
            [
                KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
                KeyValue::new(SERVICE_NAMESPACE, "scrayosnet"),
                KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, environment.to_string()),
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
    let sentry_instance = config.sentry.as_ref().map(|config| {
        sentry::init((
            config.address.clone(),
            sentry::ClientOptions {
                debug: config.debug,
                release: sentry::release_name!(),
                environment: Some(Owned(config.environment.to_string())),
                ..sentry::ClientOptions::default()
            },
        ))
    });

    // build future to execute
    let runner = async {
        // initialize opentelemetry meter (metrics)
        let meter_provider = if let Some(meter_config) = &config.otel.metrics {
            let meter_headers = HashMap::from_iter([(
                "authorization".to_string(),
                format!("Basic {}", meter_config.token),
            )]);
            let meter_exporter = MetricExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&meter_config.address)
                .with_headers(meter_headers.clone())
                .build()?;
            let meter_provider = MeterProviderBuilder::default()
                .with_periodic_exporter(meter_exporter)
                .with_resource(resource(&config.otel.environment))
                .build();
            global::set_meter_provider(meter_provider.clone());
            Some(meter_provider)
        } else {
            None
        };

        // initialize opentelemetry tracer (spans)
        let tracer_provider = if let Some(tracer_config) = &config.otel.metrics {
            let traces_headers = HashMap::from_iter([(
                "authorization".to_string(),
                format!("Basic {}", tracer_config.token),
            )]);
            let tracer_exporter = SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(&tracer_config.address)
                .with_headers(traces_headers.clone())
                .build()?;
            let tracer_provider = SdkTracerProvider::builder()
                .with_batch_exporter(tracer_exporter)
                .with_resource(resource(&config.otel.environment))
                .build();
            global::set_tracer_provider(tracer_provider.clone());
            Some(tracer_provider)
        } else {
            None
        };

        // initialize logging
        let subscriber = tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer().compact().with_filter(
                    EnvFilter::builder()
                        .with_default_directive(LevelFilter::INFO.into())
                        .from_env_lossy(),
                ),
            )
            // add optional layers
            .with(
                meter_provider
                    .as_ref()
                    .map(|provider| MetricsLayer::new(provider.clone())),
            )
            .with(
                tracer_provider
                    .as_ref()
                    .map(|provider| OpenTelemetryLayer::new(provider.tracer("passage"))),
            );

        #[cfg(feature = "sentry")]
        let subscriber = subscriber.with(sentry_tracing::layer());

        subscriber.init();
        info!(
            version = env!("CARGO_PKG_VERSION"),
            name = env!("CARGO_PKG_NAME"),
            "starting passage"
        );

        #[cfg(feature = "sentry")]
        if sentry_instance.is_some() {
            info!("sentry is enabled");
        } else {
            info!("sentry is disabled");
        }

        if config.auth_secret.is_some() {
            info!("auth cookie is enabled");
        } else {
            info!("auth cookie is disabled");
        }

        // run passage blocking
        let result = passage::start(config).await;

        // shutdown opentelemetry
        if let Some(Err(err)) = meter_provider.map(|provider| provider.shutdown()) {
            warn!(err = %err, "Error while closing meter provider");
        }
        if let Some(Err(err)) = tracer_provider.map(|provider| provider.shutdown()) {
            warn!(err = %err, "Error while closing trace provider");
        }

        result
    };

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(runner)
}
