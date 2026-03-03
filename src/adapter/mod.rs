//! Dynamic Adapters
//!
//! This module wraps the different adapter types into combined enums. By wrapping the adapters in
//! enums, we are able to select the adapter at runtime from the runtime configuration while creating
//! a statically typed [`passage_protocol::protocol::listener::Listener`].

use regex::Regex;
use uuid::Uuid;

pub mod authentication;
pub mod discovery;
pub mod filter;
pub mod localization;
pub mod status;
pub mod strategy;

/// A utility for converting an [`Option<String>`] into an [`Option<Regex>`]. Returns an error if the
/// string cannot be parsed as a regular expression.
pub(crate) fn opt_to_regex(s: Option<String>) -> Result<Option<Regex>, regex::Error> {
    if let Some(s) = s {
        return Ok(Some(Regex::new(&s)?));
    }
    Ok(None)
}

/// A utility for converting an [`Option<Vec<String>>`] into an [`Option<Vec<Uuid>>`]. Returns an
/// error if any of the strings cannot be parsed as a UUID.
pub(crate) fn opt_vec_to_uuid(ss: Option<Vec<String>>) -> Result<Option<Vec<Uuid>>, uuid::Error> {
    if let Some(ss) = ss {
        let mut result = Vec::with_capacity(ss.len());
        for s in ss {
            result.push(Uuid::parse_str(&s)?);
        }
        return Ok(Some(result));
    }
    Ok(None)
}
