#![cfg(test)]

use std::sync::{Arc, Mutex};

use opentelemetry::Value;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::Status;
use opentelemetry_sdk::trace::InMemorySpanExporter;

use crate::otel::logger_runtime::hooks::{
    RegisteredHooks, RuntimeHooks, phase_span, seed_request_identity,
};
use crate::otel::model::baggage::TEAM_ID;
use crate::otel::model::metadata::wire::payload::Auth;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::identity::IdentitySeed;
use crate::otel::v2_callback::registry::V2Registry;
use crate::otel::vocabulary::constants::attribute_keys::{error as error_keys, exception_event};

use super::support::{attribute, fixture_with_hooks, lock, register_callback, settings};

#[test]
fn absent_hooks_noop() {
    let fixture = fixture_with_hooks(None);
    let trace = RequestTraceContext::default();
    let calls = Arc::new(Mutex::new(0));
    let result = phase_span(&fixture.runtime, "phase", &trace, |span| {
        *lock(&calls) += 1;
        assert!(span.is_none());
        Ok::<u8, SpanError>(7)
    });
    assert_eq!(result, Ok(7));
    assert_eq!(*lock(&calls), 1);
    let auth = Auth::default();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: None,
    };
    let mut identity_trace = trace.clone();
    assert!(seed_request_identity(&fixture.runtime, &seed, &mut identity_trace).is_none());
}

#[test]
fn seed_identity_delegates_to_the_hook() {
    let registry = Arc::new(V2Registry::default());
    let sink = InMemorySpanExporter::default();
    let mut config = settings(vec![]);
    config.baggage_promoted_keys = vec![TEAM_ID.to_owned()];
    register_callback(&registry, config, "registered", &sink);
    let hooks: Arc<dyn RuntimeHooks> = Arc::new(RegisteredHooks {
        registry: Arc::clone(&registry),
    });
    let fixture = fixture_with_hooks(Some(hooks));
    let auth = Auth {
        team_id: Some("team-1".to_owned()),
        ..Auth::default()
    };
    let seed = IdentitySeed {
        auth: &auth,
        model: Some("gpt-4o"),
        request_metadata: None,
        ambient_span: None,
    };
    let mut trace = RequestTraceContext::default();
    let context = seed_request_identity(&fixture.runtime, &seed, &mut trace)
        .expect("the hook seeded a request context");
    assert_eq!(
        context.baggage().get(TEAM_ID).map(ToString::to_string),
        Some("team-1".to_owned())
    );
}

#[test]
fn phase_error_propagates() {
    let registry = Arc::new(V2Registry::default());
    let sink = InMemorySpanExporter::default();
    register_callback(&registry, settings(vec![]), "registered", &sink);
    let hooks: Arc<dyn RuntimeHooks> = Arc::new(RegisteredHooks {
        registry: Arc::clone(&registry),
    });
    let fixture = fixture_with_hooks(Some(hooks));
    let trace = RequestTraceContext::default();
    let error = SpanError {
        error_type: Some("Boom".to_owned()),
        message: Some("kaboom".to_owned()),
        ..SpanError::default()
    };
    let result = phase_span(&fixture.runtime, "phase", &trace, |span| {
        let phase = span.expect("the hook opened a span");
        assert!(phase.span.context.is_valid());
        Err::<u8, SpanError>(error.clone())
    });
    assert_eq!(result, Err(error));
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    let data = &spans[0];
    assert_eq!(data.name.as_ref(), "phase");
    assert_eq!(data.status, Status::error("kaboom"));
    assert_eq!(
        attribute(data, error_keys::TYPE),
        Some(&Value::from("Boom"))
    );
    assert_eq!(
        attribute(data, error_keys::MESSAGE),
        Some(&Value::from("kaboom"))
    );
    assert_eq!(data.events.events.len(), 1);
    assert_eq!(data.events.events[0].name, exception_event::NAME);
}
