#![cfg(test)]

use std::sync::{Arc, Mutex};

use opentelemetry::Value;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::Status;

use crate::otel::logger_runtime::hooks::{RegisteredHooks, phase_span, seed_request_identity};
use crate::otel::model::baggage::TEAM_ID;
use crate::otel::model::metadata::wire::payload::Auth;
use crate::otel::model::payloads::error::SpanError;
use crate::otel::plumbing::context::context_slots::RequestTraceContext;
use crate::otel::v2_callback::identity::IdentitySeed;
use crate::otel::vocabulary::constants::attribute_keys::{error as error_keys, exception_event};

use super::support::{attribute, fixture, lock, register_callback, settings};

#[test]
fn absent_hooks_noop() {
    let fixture = fixture();
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
    let mut fixture = fixture();
    let mut config = settings(vec![]);
    config.baggage_promoted_keys = vec![TEAM_ID.to_owned()];
    register_callback(&fixture, config, "registered");
    fixture.runtime.hooks = Some(Arc::new(RegisteredHooks {
        registry: Arc::clone(&fixture.registered),
    }));
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
    register_callback(&fixture, settings(vec![]), "registered");
    fixture.runtime.hooks = Some(Arc::new(RegisteredHooks {
        registry: Arc::clone(&fixture.registered),
    }));
    let trace = RequestTraceContext::default();
    let error = SpanError {
        error_type: Some("Boom".to_owned()),
        message: Some("kaboom".to_owned()),
        code: None,
        stack_trace: None,
        llm_provider: None,
    };
    let result = phase_span(&fixture.runtime, "phase", &trace, |span| {
        let phase = span.expect("the hook opened a span");
        assert!(phase.span.context.is_valid());
        Err::<u8, SpanError>(error.clone())
    });
    assert_eq!(result, Err(error));
    let spans = fixture.export_sinks.spans();
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
