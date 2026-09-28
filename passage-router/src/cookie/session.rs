use crate::cookie::{Cookie, CookieError};
use passage_core::wire::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio_util::bytes::{BufMut, BytesMut};
use uuid::Uuid;

/// The session cookie key.
pub const SESSION_COOKIE_KEY: &str = "passage:session";

/// The [`SessionCookie`] holds any additional session information about the client. This information
/// is not signed and may be tampered with by the client. Instead, it is meant to store additional
/// information supplementing the [`AuthCookie`](super::auth::AuthCookie) without the additional signature bytes and being
/// configurable without the need for the signing secret.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionCookie {
    /// The ID of the session.
    pub id: Uuid,

    /// The address of the server, the client (initially) connected to.
    pub server_address: String,

    /// The port of the server, the client (initially) connected to.
    pub server_port: u16,

    /// Any additional system-specific (unsecured) information. This includes the OpenTelemetry tracing
    /// information.
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl Cookie for SessionCookie {
    const KEY: &'static str = SESSION_COOKIE_KEY;

    fn encode(&self, _: Option<&[u8]>) -> Result<Bytes, CookieError> {
        let mut bytes = BytesMut::with_capacity(64);
        serde_json::to_writer((&mut bytes).writer(), self)?;
        Ok(bytes.freeze())
    }

    fn decode(_: Option<&[u8]>, signed: &[u8]) -> Result<Option<Self>, CookieError> {
        Ok(Some(serde_json::from_slice(signed)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::propagation::TextMapPropagator;
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry_sdk::propagation::TraceContextPropagator;

    #[test]
    fn a_session_carries_a_trace_across_the_transfer_it_survives() {
        // What the link in `on_login_encryption_response` depends on: `extra` is the only thing
        // tying a transferred player's connection back to the login that issued the session, and a
        // serde change that dropped it would break that silently rather than loudly.
        let propagator = TraceContextPropagator::new();
        let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let extra = HashMap::from([("traceparent".to_owned(), traceparent.to_owned())]);

        let cookie = SessionCookie {
            id: Uuid::from_u128(7),
            server_address: "mc.justchunks.net".to_owned(),
            server_port: 25_565,
            extra,
        };
        let encoded = cookie.encode(None).expect("encodes");
        let decoded = SessionCookie::decode(None, &encoded)
            .expect("decodes")
            .expect("a session");

        assert_eq!(decoded.id, Uuid::from_u128(7));
        let context = propagator.extract(&decoded.extra);
        let span = context.span().span_context().clone();
        assert!(span.is_valid(), "the trace has to survive the round trip");
        assert_eq!(
            span.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736",
        );
        assert_eq!(span.span_id().to_string(), "00f067aa0ba902b7");
    }

    #[test]
    fn a_session_without_a_trace_is_not_linked_to_one() {
        // The guard the link is written behind: a client may hand back a session from a deployment
        // that exported no traces at all, and an invalid context must not be linked.
        let propagator = TraceContextPropagator::new();
        let cookie = SessionCookie {
            id: Uuid::from_u128(1),
            ..SessionCookie::default()
        };
        let encoded = cookie.encode(None).expect("encodes");
        let decoded = SessionCookie::decode(None, &encoded)
            .expect("decodes")
            .expect("a session");

        assert!(decoded.extra.is_empty());
        let context = propagator.extract(&decoded.extra);
        assert!(!context.span().span_context().is_valid());
    }
}
