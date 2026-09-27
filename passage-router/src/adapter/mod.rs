//! The adapters a route answers with, and the runtime-selected enums that wrap them.
//!
//! Each submodule holds one `Dyn*Adapter`: an enum over every built-in and feature-gated
//! implementation of one adapter trait, so that which implementation answers is a configuration
//! decision rather than a type parameter the whole crate has to carry.

use regex::Regex;
use uuid::Uuid;

mod adapter;
#[cfg(test)]
pub mod held;

pub mod authentication;
pub mod discovery;
pub mod localization;
pub mod status;

pub use adapter::{Route, Routes};

pub(crate) fn opt_to_regex(s: Option<String>) -> Result<Option<Regex>, regex::Error> {
    if let Some(s) = s {
        return Ok(Some(Regex::new(&s)?));
    }
    Ok(None)
}

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
