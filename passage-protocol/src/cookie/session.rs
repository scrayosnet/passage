use crate::cookie::Cookie;
use opentelemetry::trace::SpanContext;
use opentelemetry::{SpanId, TraceFlags, TraceId};
use serde::{Deserialize, Serialize};
use tracing::trace;
use uuid::Uuid;

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionCookie {
    pub id: Uuid,
    pub server_address: String,
    pub server_port: u16,
    #[serde(default)]
    pub trace_id: Option<String>,
}

impl SessionCookie {
    pub fn span_cx(&self) -> Option<SpanContext> {
        // get trace id and parse
        let Some(trace_id) = &self.trace_id else {
            trace!("no trace id set in session cookie");
            return None;
        };

        let Ok(trace_id) = TraceId::from_hex(trace_id) else {
            trace!("failed to parse trace id from session cookie");
            return None;
        };

        // create span context from trace id
        Some(SpanContext::new(
            trace_id,
            SpanId::INVALID,
            TraceFlags::SAMPLED,
            true,
            Default::default(),
        ))
    }
}

impl Cookie for SessionCookie {
    const KEY: &'static str = SESSION_COOKIE_KEY;
}
