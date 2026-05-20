use passage::config::Config;

/// Prints the JSON schema of the application config.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", Config::schema()?);
    Ok(())
}
