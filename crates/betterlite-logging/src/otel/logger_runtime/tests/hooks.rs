#![cfg(test)]

use std::sync::{Arc, Mutex};

use opentelemetry::Context;
use opentelemetry::Value;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    Span as _, Status, TraceContextExt as _, Tracer as _, TracerProvider as _,
};

use crate::otel::logger_runtime::hooks::{phase_span, seed_request_identity};
use crate::otel::model::baggage::TEAM_ID;
use crate::otel::model::metadata::wire::payload::Auth;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::identity::IdentitySeed;
use crate::otel::vocabulary::constants::attribute_keys::{error as error_keys, exception_event};

use super::support::{Fixture, attribute, fixture, lock, register_callback, settings};

#[test]
fn absent_hooks_noop() {
    let fixture = fixture();
    let trace = RequestTraceContext::default();
    let calls = Arc::new(Mutex::new(0_u32));
    let result = phase_span(&fixture.runtime, "phase", &trace, |span| {
        *lock(&calls) += 1_u32;
        assert!(span.is_none());
        Ok::<u8, SpanError>(7)
    });
    assert_eq!(result, Ok(7));
    assert_eq!(*lock(&calls), 1_u32);
    let auth = Auth::default();
    let seed = IdentitySeed {
        auth: &auth,
        model: None,
        request_metadata: None,
        ambient_span: None,
    };
    let mut identity_trace = trace;
    assert!(seed_request_identity(&fixture.runtime, &seed, &mut identity_trace).is_none());
}

#[test]
fn seed_identity_delegates_to_the_hook() {
    let mut fixture = fixture();
    let mut config = settings(vec![]);
    config.baggage_promoted_keys = vec![TEAM_ID.to_owned()];
    register_callback(&fixture, config, "registered");
    let hooks = Arc::clone(&fixture.registered);
    fixture.runtime.hooks = Some(hooks);
    let auth = Auth {
        team_id: Some("team-1".to_owned()),
        team_alias: None,
        team_metadata: None,
        key_hash: None,
        end_user_id: None,
        user_id: None,
        org_id: None,
        key_alias: None,
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
    let mut fixture = fixture();
    let callback = register_callback(&fixture, settings(vec![]), "registered");
    let hooks = Arc::clone(&fixture.registered);
    fixture.runtime.hooks = Some(hooks);
    let trace = RequestTraceContext::default();
    let error = SpanError {
        error_type: Some("Boom".to_owned()),
        message: Some("kaboom".to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    };
    assert!(!Context::current().has_active_span());
    let result = phase_span(&fixture.runtime, "phase", &trace, |span| {
        let phase = span.expect("the hook opened a span");
        assert!(phase.span.context.is_valid());
        // attach() made the phase current, so a child span starts under it
        let current = Context::current();
        assert_eq!(
            current.span().span_context().span_id(),
            phase.span.context.span_id()
        );
        let mut child = callback.tracer_provider.tracer("phase-test").start("child");
        child.end();
        Err::<u8, SpanError>(error.clone())
    });
    assert_eq!(result, Err(error));
    // the guard is dropped with the phase, so the caller's context is untouched
    assert!(!Context::current().has_active_span());
    assert_phase_exported(&fixture);
}

fn assert_phase_exported(fixture: &Fixture) {
    let spans = fixture.export_sinks.spans();
    assert_eq!(spans.len(), 2);
    let phase = spans
        .iter()
        .find(|data| data.name.as_ref() == "phase")
        .expect("the phase span");
    let child = spans
        .iter()
        .find(|data| data.name.as_ref() == "child")
        .expect("the child span");
    assert_eq!(child.parent_span_id, phase.span_context.span_id());
    assert_eq!(child.span_context.trace_id(), phase.span_context.trace_id());
    assert_eq!(phase.status, Status::error("kaboom"));
    assert_eq!(
        attribute(phase, error_keys::TYPE),
        Some(&Value::from("Boom"))
    );
    assert_eq!(
        attribute(phase, error_keys::MESSAGE),
        Some(&Value::from("kaboom"))
    );
    assert_eq!(phase.events.events.len(), 1);
    assert_eq!(phase.events.events[0].name, exception_event::NAME);
}
