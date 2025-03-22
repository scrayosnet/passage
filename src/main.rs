use passage::config::Config;
use std::sync::Arc;
use tracing_subscriber::prelude::*;

/// Initializes the application and invokes passage.
///
/// This initializes the logging, aggregates configuration and starts the multithreaded tokio runtime. This is only a
/// thin-wrapper around the passage crate that supplies the necessary settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse the arguments and configuration
    let app_settings = Config::new()?;

    // initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact())
        .with(app_settings.logging.level.clone().0)
        .init();

    // initialize the application state
    let state = Arc::new(app_settings);

    // run passage blocking
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { passage::start(state).await })
}
