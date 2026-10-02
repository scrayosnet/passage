use crate::cookie::{Cookie, CookieError};
use opentelemetry::trace::{SpanContext, TraceContextExt};
use opentelemetry::{Context, global};
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

impl SessionCookie {
    /// Generates a new session with a random id.
    pub fn new(server_address: impl Into<String>, server_port: u16) -> Self {
        Self {
            id: Uuid::new_v4(),
            server_address: server_address.into(),
            server_port,
            extra: Default::default(),
        }
    }

    /// The trace of the hop that issued this session, if it carries a valid one.
    pub fn linked_context(&self) -> Option<SpanContext> {
        let context = global::get_text_map_propagator(|propagator| {
            propagator.extract_with_context(&Context::new(), &self.extra)
        });
        let span_context = context.span().span_context().clone();
        span_context.is_valid().then_some(span_context)
    }

    /// Puts the trace of `context` into the session, for the next hop to link back to. This overwrites
    /// any existing context. Keeps the original context if this context does not trace.
    pub fn set_trace(&mut self, context: &Context) {
        global::get_text_map_propagator(|propagator| {
            propagator.inject_context(context, &mut self.extra);
        });
    }
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
    use opentelemetry::trace::{
        SpanId, TraceContextExt, TraceFlags, TraceId, TraceState, TracerProvider,
    };
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_opentelemetry::OpenTelemetryLayer;
    use tracing_subscriber::prelude::*;

    /// A traceparent naming a hop that is not this process, for the cookie to arrive carrying.
    const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    /// Installs the W3C propagator, which is what production does when traces are exported.
    ///
    /// Process-global and therefore not undone: every test here wants it, and the rest of the crate
    /// has no valid context to propagate, so a real propagator behaves as a no-op one for them.
    fn with_propagator() {
        global::set_text_map_propagator(TraceContextPropagator::new());
    }

    /// Runs `f` inside an entered span whose OpenTelemetry context is active, the way a handler
    /// runs under `OpenTelemetryLayer`. This is the condition the fallback below only shows up in.
    fn in_an_active_span(f: impl FnOnce()) {
        let provider = SdkTracerProvider::builder().build();
        let subscriber =
            tracing_subscriber::registry().with(OpenTelemetryLayer::new(provider.tracer("test")));
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("on_login_encryption_response");
            let _entered = span.enter();
            assert!(
                Context::current().span().span_context().is_valid(),
                "the test proves nothing unless a span is really active",
            );
            f();
        });
    }

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
    fn a_session_without_a_trace_is_not_linked_to_the_span_that_asks_for_it() {
        // The regression, and the reason this is not simply `extract`: a propagator that finds no
        // `traceparent` hands back the context it was given, and `extract` gives it
        // `Context::current()` -- which, under an `OpenTelemetryLayer`, is the span the caller is
        // standing in. A session with no trace used to answer with the *asking handler's* span, so
        // the connection span linked itself to its own child, inside its own trace. Grafana reads a
        // link as a parent reference for a span that has none, so the tree closed into a cycle and
        // the whole trace rendered as nothing at all.
        //
        // Every player that logs in without a session hits this, which is why minting one for
        // everybody is what brought it out.
        with_propagator();
        in_an_active_span(|| {
            let session = SessionCookie::new("mc.justchunks.net", 25_565);
            assert!(session.extra.is_empty(), "a fresh session carries no trace");
            assert!(
                session.linked_context().is_none(),
                "a trace-less session must not be linked to the span that asks it",
            );
        });
    }

    #[test]
    fn a_session_is_linked_to_the_hop_that_issued_it_and_not_to_the_span_that_asks() {
        // The other half: a session that really does carry a trace answers with that one, and not
        // with the ambient span that happens to be entered when it is asked.
        with_propagator();
        in_an_active_span(|| {
            let session = SessionCookie {
                extra: HashMap::from([("traceparent".to_owned(), TRACEPARENT.to_owned())]),
                ..SessionCookie::new("mc.justchunks.net", 25_565)
            };
            let linked = session.linked_context().expect("a trace to link");
            assert_eq!(
                linked.trace_id().to_string(),
                "4bf92f3577b34da6a3ce929d0e0e4736",
            );
            assert_eq!(linked.span_id().to_string(), "00f067aa0ba902b7");
        });
    }

    #[test]
    fn a_hop_that_exports_no_trace_passes_the_previous_one_through() {
        // A hop with tracing switched off has no valid context, and a propagator given one injects
        // nothing. What it already carries is then left alone on purpose: this hop emits no span
        // for anyone to link to, so the nearest hop that did is the most useful thing the next one
        // can find. Clearing it here would cut the chain at a hop that is invisible either way.
        with_propagator();
        let mut session = SessionCookie {
            extra: HashMap::from([("traceparent".to_owned(), TRACEPARENT.to_owned())]),
            ..SessionCookie::new("mc.justchunks.net", 25_565)
        };

        session.set_trace(&Context::new());

        let linked = session
            .linked_context()
            .expect("the last hop that had a trace stays linkable through this one");
        assert_eq!(
            linked.span_id().to_string(),
            "00f067aa0ba902b7",
            "an untraced hop passes the chain on rather than ending it",
        );
    }

    #[test]
    fn a_hop_replaces_the_trace_the_session_arrived_with() {
        // The refresh the chain depends on: each hop that traces hands the session on carrying its
        // own trace, so the next one links to where it actually came from rather than to the first
        // in the chain. No clearing is needed for that -- injecting overwrites every field the
        // propagator owns -- and nothing else in `extra` is touched.
        with_propagator();
        let mut session = SessionCookie {
            extra: HashMap::from([
                ("traceparent".to_owned(), TRACEPARENT.to_owned()),
                ("unrelated".to_owned(), "kept".to_owned()),
            ]),
            ..SessionCookie::new("mc.justchunks.net", 25_565)
        };

        let this_hop = Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").expect("a trace id"),
            SpanId::from_hex("b7ad6b7169203331").expect("a span id"),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));
        session.set_trace(&this_hop);

        let linked = session.linked_context().expect("this hop's trace");
        assert_eq!(
            linked.trace_id().to_string(),
            "0af7651916cd43dd8448eb211c80319c",
            "the session carries this hop's trace, not the one it arrived with",
        );
        assert_eq!(linked.span_id().to_string(), "b7ad6b7169203331");
        assert_eq!(
            session.extra.get("unrelated").map(String::as_str),
            Some("kept"),
            "only the trace fields are ours to clear",
        );
    }
}
