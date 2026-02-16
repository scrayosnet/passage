use regex::Regex;
use uuid::Uuid;

pub mod authentication;
pub mod discovery;
pub mod filter;
pub mod localization;
pub mod status;
pub mod strategy;

pub(crate) fn opt_to_regex(s: Option<String>) -> Result<Option<Regex>, regex::Error> {
    if let Some(s) = s {
        return Ok(Some(Regex::new(&s)?));
    }
    Ok(None)
}

pub(crate) fn opt_vec_to_regex(
    ss: Option<Vec<String>>,
) -> Result<Option<Vec<Regex>>, regex::Error> {
    if let Some(ss) = ss {
        let mut result = Vec::with_capacity(ss.len());
        for s in ss {
            result.push(Regex::new(&s)?);
        }
        return Ok(Some(result));
    }
    Ok(None)
}

pub(crate) fn opt_to_uuid(s: Option<String>) -> Result<Option<Uuid>, uuid::Error> {
    if let Some(s) = s {
        return Ok(Some(Uuid::parse_str(&s)?));
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
