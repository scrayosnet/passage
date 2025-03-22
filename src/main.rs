use passage::config::Config;
use std::borrow::Cow::Owned;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::prelude::*;

/// Initializes the application and invokes passage.
///
/// This initializes the logging, aggregates configuration and starts the multithreaded tokio runtime. This is only a
/// thin-wrapper around the passage crate that supplies the necessary settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse the arguments and configuration
    let settings = Config::new()?;

    // initialize sentry
    let _sentry = sentry::init((
        settings
            .sentry
            .enabled
            .then_some(settings.sentry.address.clone()),
        sentry::ClientOptions {
            debug: settings.sentry.debug,
            release: sentry::release_name!(),
            environment: Some(Owned(settings.sentry.environment.clone())),
            ..sentry::ClientOptions::default()
        },
    ));

    // initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact())
        .with(sentry_tracing::layer())
        .with(settings.logging.level.clone().0)
        .init();

    if _sentry.is_enabled() {
        info!("sentry is enabled");
    }

    // initialize the application state
    let state = Arc::new(settings);

    // run passage blocking
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { passage::start(state).await })
}
