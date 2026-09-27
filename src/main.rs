//! The Passage binary.
//!
//! Everything Passage does lives in [`passage_router`]; this is only the wiring that reads the
//! configuration, stands the telemetry up, and hands both to the server.

use passage_router::config::Config;

/// Reads the configuration, initializes telemetry, and runs Passage until it is asked to stop.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::read()?;

    // Held for the rest of `main`: dropping it is what flushes the exporters, so it has to outlive
    // the server it is recording.
    let _telemetry = passage_router::init_tracing(&config)?;

    passage_router::start(config).await
}
