#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::trace::{SpanId, Status};

use betterlite_logging::otel::model::payloads::service::ServiceType;
use betterlite_logging::otel::plumbing::context::post_response::ServiceSpanTiming;
use betterlite_logging::otel::v2_callback::service::ServiceHook;

use super::support::{attribute, harness, hook, service_payload, settings, timing};

#[test]
fn success_parents_to_the_threaded_span_and_labels_the_service() {
    let harness = harness(settings());
    let root = harness.span("root");
    let payload = service_payload(ServiceType::Redis, "get");
    let service_hook = hook(&payload, Some(&root.context), None, timing(None));
    harness.callback.async_service_success_hook(&service_hook);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name.as_ref(), "redis get");
    assert_eq!(
        attribute(&spans[0], "litellm.service.name").as_deref(),
        Some("redis")
    );
    assert_eq!(
        attribute(&spans[0], "litellm.service.call_type").as_deref(),
        Some("get")
    );
    assert_eq!(
        attribute(&spans[0], "litellm.service.caller").as_deref(),
        Some("test")
    );
    assert_eq!(spans[0].parent_span_id, root.context.span_id());
}

#[test]
fn a_failure_without_a_payload_error_stamps_the_hook_text() {
    let harness = harness(settings());
    let payload = service_payload(ServiceType::Redis, "set");
    let service_hook = hook(&payload, None, None, timing(None));
    harness
        .callback
        .async_service_failure_hook(&service_hook, Some("boom"));
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("boom")
    );
}

#[test]
fn a_failure_without_any_text_still_stamps_an_error() {
    let harness = harness(settings());
    let payload = service_payload(ServiceType::Redis, "set");
    let service_hook = hook(&payload, None, None, timing(None));
    harness
        .callback
        .async_service_failure_hook(&service_hook, None);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("error")
    );
}

#[test]
fn the_payload_error_wins_over_the_hook_text() {
    let harness = harness(settings());
    let mut payload = service_payload(ServiceType::Redis, "set");
    payload.error = Some("payload-err".to_owned());
    let service_hook = hook(&payload, None, None, timing(None));
    harness
        .callback
        .async_service_failure_hook(&service_hook, Some("hook-err"));
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("payload-err")
    );
}

#[test]
fn metrics_only_services_emit_no_span() {
    let harness = harness(settings());
    let payload = service_payload(ServiceType::SelfService, "completion");
    let service_hook = hook(&payload, None, None, timing(None));
    harness
        .callback
        .async_service_failure_hook(&service_hook, Some("boom"));
    assert!(harness.finished().is_empty());
}

#[test]
fn a_carrierless_ping_emits_nothing() {
    let harness = harness(settings());
    let payload = service_payload(ServiceType::Redis, "ping");
    let service_hook = hook(&payload, None, None, timing(None));
    harness.callback.async_service_success_hook(&service_hook);
    assert!(harness.finished().is_empty());
}

#[test]
fn background_timing_emits_a_root_span() {
    let harness = harness(settings());
    let start = SystemTime::now();
    let end = start
        .checked_add(Duration::from_millis(5))
        .expect("end after start");
    let payload = service_payload(ServiceType::Redis, "flush");
    let service_hook = hook(&payload, None, Some(start), timing(Some(end)));
    harness.callback.async_service_success_hook(&service_hook);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
}

#[test]
fn a_parent_that_ended_before_the_call_becomes_a_link() {
    let harness = harness(settings());
    let root = harness.span("root");
    let parent_end = UNIX_EPOCH
        .checked_add(Duration::from_secs(10))
        .expect("parent end");
    let end = parent_end
        .checked_add(Duration::from_secs(5))
        .expect("end after the parent");
    let payload = service_payload(ServiceType::Redis, "get");
    let service_hook = ServiceHook {
        payload: &payload,
        parent_otel_span: Some(&root.context),
        start_time: Some(parent_end),
        timing: ServiceSpanTiming {
            end_time: Some(end),
            parent_end_time: Some(parent_end),
            post_response: false,
        },
        event_metadata: None,
    };
    harness.callback.async_service_success_hook(&service_hook);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
    assert_eq!(spans[0].links.links.len(), 1);
    assert_eq!(
        spans[0].links.links[0].span_context.span_id(),
        root.context.span_id()
    );
}
