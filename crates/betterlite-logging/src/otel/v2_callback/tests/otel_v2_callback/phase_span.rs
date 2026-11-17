#![cfg(test)]

use opentelemetry::trace::{Span as _, Status};

use betterlite_logging::otel::model::payloads::service::ServiceType;
use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::v2_callback::phase::start_phase_span;
use betterlite_logging::otel::v2_callback::proxy;
use betterlite_logging::otel::v2_callback::registered;
use betterlite_logging::otel::v2_callback::registry::V2Registry;
use betterlite_logging::otel::v2_callback::service;

use super::support::{
    attribute, error, event_names, harness, hook, service_payload, settings, timing,
};

#[test]
fn dropping_the_phase_span_ends_it() {
    let harness = harness(settings());
    let phase = start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    assert!(phase.span.lock().is_recording());
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
    assert_eq!(spans[0].status, Status::Unset);
}

#[test]
fn the_phase_span_parents_to_the_anchored_root() {
    let harness = harness(settings());
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    let phase = start_phase_span(&harness.callback, "auth", &trace);
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
    assert_eq!(spans[0].parent_span_id, root.context.span_id());
}

#[test]
fn a_service_span_inside_the_phase_nests_under_it() {
    let harness = harness(settings());
    let root = harness.span("root");
    let phase = start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    let guard = phase.attach();
    let payload = service_payload(ServiceType::Redis, "get");
    let service_hook = hook(&payload, Some(&root.context), None, timing(None), None);
    let _emitted = service::emit_service(&harness.callback, &service_hook, None);
    let phase_id = phase.span.context.span_id();
    drop(guard);
    drop(phase);
    let spans = harness.finished();
    let nested = spans
        .iter()
        .find(|span| span.name.as_ref() == "redis get")
        .expect("the service span");
    assert_eq!(nested.parent_span_id, phase_id);
}

#[test]
fn recording_an_error_stamps_the_status_and_one_event() {
    let harness = harness(settings());
    let phase = start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    phase.record_error(&error("boom"));
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("boom")
    );
}

#[test]
fn recording_an_error_on_an_ended_span_changes_nothing() {
    let harness = harness(settings());
    let phase = start_phase_span(&harness.callback, "auth", &RequestTraceContext::default());
    phase.span.lock().end();
    phase.record_error(&error("boom"));
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].status, Status::Unset);
    assert!(spans[0].events.is_empty());
}

#[test]
fn the_module_entry_returns_none_without_a_logger() {
    let registry = V2Registry::default();
    assert!(registered::phase_span(&registry, "auth", &RequestTraceContext::default()).is_none());
}

#[test]
fn the_module_entry_routes_to_the_registered_logger() {
    let harness = harness(settings());
    let phase = registered::phase_span(&harness.registry, "auth", &RequestTraceContext::default())
        .expect("registered");
    drop(phase);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "auth");
}
