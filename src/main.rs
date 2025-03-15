use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::prelude::*;

/// Arguments to configure this runtime of the application before it is started.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, env, default_value = "INFO")]
    log_level: LevelFilter,
    #[arg(long, env, default_value_t = SocketAddr::from(passage::config::DEFAULT_ADDRESS))]
    address: SocketAddr,
    #[arg(long, env, default_value_t = passage::config::DEFAULT_TIMEOUT_SECS)]
    timeout: f64,
    #[arg(long, env, default_value_t = passage::config::DEFAULT_KEY_LENGTH)]
    key_length: u32,
}

/// Initializes the application and invokes passage.
///
/// This initializes the logging, aggregates configuration and starts the multithreaded tokio runtime. This is only a
/// thin-wrapper around the passage crate that supplies the necessary settings.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // parse the arguments and configuration
    let args = Args::parse();

    // initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_filter(args.log_level),
        )
        .init();

    // initialize the application state
    let state = Arc::new(passage::config::AppState::new(
        args.address,
        args.timeout,
        args.key_length,
    ));

    // run passage blocking
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { passage::start(state).await })
}
