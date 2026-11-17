#![cfg(test)]

use opentelemetry::trace::Status;

use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, McpTransportSpan, RequestTraceContext,
};

use super::support::{attribute, event_names, exception_facts, harness, settings};

fn exported<'a>(
    spans: &'a [opentelemetry_sdk::trace::SpanData],
    name: &str,
) -> &'a opentelemetry_sdk::trace::SpanData {
    spans
        .iter()
        .find(|span| span.name.as_ref() == name)
        .expect("the named span was exported")
}

#[test]
fn the_response_status_pins_the_error_code() {
    let harness = harness(settings());
    let span = harness.span("server");
    let facts = exception_facts(401);
    harness
        .callback
        .record_error_attributes_on_span(Some(&span), Some(&facts), 400);
    span.end();
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(
        attribute(&spans[0], "litellm.provider.error.code").as_deref(),
        Some("400")
    );
    assert_eq!(
        attribute(&spans[0], "error.type").as_deref(),
        Some("AuthenticationError")
    );
    assert_eq!(
        attribute(&spans[0], "error.message").as_deref(),
        Some("bad credentials")
    );
    assert_eq!(
        attribute(&spans[0], "litellm.provider.error.llm_provider").as_deref(),
        Some("openai")
    );
}

#[test]
fn a_second_stamp_keeps_one_event_and_repins_the_code() {
    let harness = harness(settings());
    let span = harness.span("server");
    let facts = exception_facts(401);
    harness
        .callback
        .record_error_attributes_on_span(Some(&span), Some(&facts), 400);
    harness
        .callback
        .record_error_attributes_on_span(Some(&span), Some(&facts), 422);
    span.end();
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
    assert_eq!(
        attribute(&spans[0], "litellm.provider.error.code").as_deref(),
        Some("422")
    );
}

#[test]
fn an_ended_span_is_not_stamped() {
    let harness = harness(settings());
    let span = harness.span("server");
    span.end();
    let facts = exception_facts(401);
    harness
        .callback
        .record_error_attributes_on_span(Some(&span), Some(&facts), 400);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(attribute(&spans[0], "litellm.provider.error.code").is_none());
    assert!(spans[0].events.is_empty());
}

#[test]
fn a_missing_span_is_a_no_op() {
    let harness = harness(settings());
    let facts = exception_facts(401);
    harness
        .callback
        .record_error_attributes_on_span(None, Some(&facts), 400);
    assert!(harness.finished().is_empty());
}

#[test]
fn the_msg_transport_span_beats_the_root() {
    let harness = harness(settings());
    let transport = harness.span("transport");
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    harness
        .callback
        .mark_request_root_span(&transport, None, &mut trace);
    harness
        .callback
        .mark_request_root_span(&root, None, &mut trace);
    trace.mcp_transport = Some(McpTransportSpan {
        span: AnchoredSpan {
            context: transport.context.clone(),
            http_route: None,
        },
        recording: true,
    });
    let facts = exception_facts(400);
    harness
        .callback
        .async_post_call_failure_hook(Some(&facts), None, None, &trace);
    transport.end();
    root.end();
    let spans = harness.finished();
    let stamped = exported(&spans, "transport");
    assert!(matches!(stamped.status, Status::Error { .. }));
    assert_eq!(
        attribute(stamped, "error.type").as_deref(),
        Some("AuthenticationError")
    );
    assert!(attribute(exported(&spans, "root"), "error.type").is_none());
}

#[test]
fn the_root_is_stamped_without_a_transport() {
    let harness = harness(settings());
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    harness
        .callback
        .mark_request_root_span(&root, None, &mut trace);
    let facts = exception_facts(403);
    harness
        .callback
        .async_post_call_failure_hook(Some(&facts), None, None, &trace);
    root.end();
    let spans = harness.finished();
    assert_eq!(
        attribute(&spans[0], "error.type").as_deref(),
        Some("AuthenticationError")
    );
}

#[test]
fn the_parent_span_is_the_last_fallback() {
    let harness = harness(settings());
    let parent = harness.span("parent");
    let trace = RequestTraceContext::default();
    let facts = exception_facts(403);
    harness
        .callback
        .async_post_call_failure_hook(Some(&facts), None, Some(&parent), &trace);
    parent.end();
    let spans = harness.finished();
    assert_eq!(
        attribute(&spans[0], "error.type").as_deref(),
        Some("AuthenticationError")
    );
}
