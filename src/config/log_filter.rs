use serde::de::{Error, Unexpected, Visitor};
use serde::Deserializer;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use tracing_subscriber::EnvFilter;

/// [`LogFilter`] is a wrapper for [`EnvFilter`].
#[derive(Debug)]
pub struct LogFilter(pub EnvFilter);

impl Display for LogFilter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Clone for LogFilter {
    fn clone(&self) -> Self {
        LogFilter(EnvFilter::from(&self.0.to_string()))
    }
}

impl FromStr for LogFilter {
    type Err = tracing_subscriber::filter::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(LogFilter(EnvFilter::try_new(s)?))
    }
}

/// Deserializer for [LogFilter] from string. E.g. `info`.
pub fn parse_level_filter<'de, D>(deserializer: D) -> Result<LogFilter, D::Error>
where
    D: Deserializer<'de>,
{
    struct LogFilterVisitor;

    impl Visitor<'_> for LogFilterVisitor {
        type Value = LogFilter;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            write!(formatter, "a log level name or number")
        }

        fn visit_str<E>(self, value: &str) -> Result<LogFilter, E>
        where
            E: Error,
        {
            match LogFilter::from_str(value) {
                Ok(filter) => Ok(filter),
                Err(_) => Err(Error::invalid_value(
                    Unexpected::Str(value),
                    &"log level string or number",
                )),
            }
        }
    }

    deserializer.deserialize_str(LogFilterVisitor)
}
