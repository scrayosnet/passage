//! Renders the JSON schema of the Passage configuration to stdout.
//!
//! The result is checked in as `config/schema.json`, which is what editors validate a deployment's
//! YAML against.

use passage_router::config::Config;

/// Prints the JSON schema of the application config.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", Config::schema()?);
    Ok(())
}
