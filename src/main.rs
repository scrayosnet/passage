use passage::config::Config;
use std::borrow::Cow::Owned;
use tracing::info;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// Initializes the application and invokes passage.
///
/// This initializes the logging, aggregates configuration and starts the multithreaded tokio runtime. This is only a
/// thin-wrapper around the passage crate that supplies the necessary settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse the arguments and configuration
    let config = Config::new()?;

    // initialize sentry
    #[cfg(feature = "sentry")]
    let _sentry = sentry::init((
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

    // initialize logging
    let subscriber =
        tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().compact());

    #[cfg(feature = "sentry")]
    let subscriber = subscriber.with(sentry_tracing::layer());

    let subscriber = subscriber.with(EnvFilter::from_default_env());

    subscriber.init();

    #[cfg(feature = "sentry")]
    if _sentry.is_enabled() {
        info!("sentry is enabled");
    }

    if config.auth_secret.is_some() {
        info!("auth cookie is enabled");
    }

    let locale = config.localization.localize_default("locale", &[]);
    info!(locale = locale, "using localization");

    // run passage blocking
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { passage::start(config).await })
}
