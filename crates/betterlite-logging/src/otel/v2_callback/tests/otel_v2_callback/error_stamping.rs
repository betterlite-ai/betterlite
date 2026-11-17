#![cfg(test)]

use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, Status};
use opentelemetry_sdk::trace::SpanData;

use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::otel::model::payloads::error::SpanError;
use betterlite_logging::otel::plumbing::context::context_slots::{
    AnchoredSpan, McpTransportSpan, RequestTraceContext,
};

use betterlite_logging::otel::v2_callback::proxy;
use betterlite_logging::otel::v2_callback::span_error::{self, ErrorInformationConfig};

use super::support::{attribute, event_names, exception_facts, harness, settings};

fn exported<'data>(spans: &'data [SpanData], name: &str) -> &'data SpanData {
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
    proxy::record_error_on_span(&harness.callback, Some(&span), Some(&facts), 400);
    span.lock().end();
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(matches!(spans[0].status, Status::Error { .. }));
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
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
    proxy::record_error_on_span(&harness.callback, Some(&span), Some(&facts), 400);
    proxy::record_error_on_span(&harness.callback, Some(&span), Some(&facts), 422);
    span.lock().end();
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
    assert_eq!(
        attribute(&spans[0], "litellm.provider.error.code").as_deref(),
        Some("422")
    );
}

#[test]
fn a_provider_error_keeps_its_traceback_and_a_client_error_drops_it() {
    let harness = harness(settings());
    let traceback = "Traceback (most recent call last):\n".to_owned();
    let client = harness.span("client");
    let mut client_facts = exception_facts(400);
    client_facts.llm_provider = None;
    client_facts.traceback_lines = vec![traceback.clone()];
    proxy::record_error_on_span(&harness.callback, Some(&client), Some(&client_facts), 400);
    client.lock().end();
    let provider = harness.span("provider");
    let mut provider_facts = exception_facts(500);
    provider_facts.llm_provider = None;
    provider_facts.traceback_lines = vec![traceback.clone()];
    proxy::record_error_on_span(
        &harness.callback,
        Some(&provider),
        Some(&provider_facts),
        500,
    );
    provider.lock().end();
    let spans = harness.finished();
    let trace_key = "litellm.provider.error.stack_trace";
    assert!(attribute(exported(&spans, "client"), trace_key).is_none());
    assert_eq!(
        attribute(exported(&spans, "provider"), trace_key).as_deref(),
        Some(traceback.as_str())
    );
}

#[test]
fn an_unrelated_attribute_does_not_hide_the_error_event() {
    let harness = harness(settings());
    let span = harness.span("server");
    span.lock()
        .set_attribute(KeyValue::new("litellm.service.name", "redis"));
    let facts = exception_facts(401);
    proxy::record_error_on_span(&harness.callback, Some(&span), Some(&facts), 400);
    span.lock().end();
    let spans = harness.finished();
    assert_eq!(event_names(&spans[0]), vec!["exception".to_owned()]);
    assert_eq!(
        attribute(&spans[0], "error.type").as_deref(),
        Some("AuthenticationError")
    );
}

#[test]
fn an_empty_exception_field_stamps_nothing() {
    let config = ErrorInformationConfig {
        normalizer: Normalizer::new().expect("normalizer"),
        redactor: None,
        log_client_error_tracebacks: false,
    };
    let facts = ExceptionFacts::default();
    let error = span_error::span_error_from_exception(&config, Some(&facts), None, None);
    assert_eq!(error, SpanError::default());
}

#[test]
fn an_ended_span_is_not_stamped() {
    let harness = harness(settings());
    let span = harness.span("server");
    span.lock().end();
    let facts = exception_facts(401);
    proxy::record_error_on_span(&harness.callback, Some(&span), Some(&facts), 400);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert!(attribute(&spans[0], "litellm.provider.error.code").is_none());
    assert!(spans[0].events.is_empty());
}

#[test]
fn a_missing_span_is_a_no_op() {
    let harness = harness(settings());
    let facts = exception_facts(401);
    proxy::record_error_on_span(&harness.callback, None, Some(&facts), 400);
    assert!(harness.finished().is_empty());
}

#[test]
fn the_msg_transport_span_beats_the_root() {
    let harness = harness(settings());
    let transport = harness.span("transport");
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &transport, None, &mut trace);
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    trace.mcp_transport = Some(McpTransportSpan {
        span: AnchoredSpan {
            context: transport.context.clone(),
            http_route: None,
        },
        recording: true,
    });
    let facts = exception_facts(400);
    proxy::async_post_call_failure_hook(&harness.callback, Some(&facts), None, None, &trace);
    transport.lock().end();
    root.lock().end();
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
fn an_anchored_root_is_found_among_several() {
    let harness = harness(settings());
    let first = harness.span("first");
    let second = harness.span("second");
    let mut first_trace = RequestTraceContext::default();
    let mut second_trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &first, None, &mut first_trace);
    proxy::mark_request_root_span(&harness.callback, &second, None, &mut second_trace);
    let found = proxy::request_root_span(&harness.callback, &second_trace);
    assert_eq!(
        found.map(|span| span.context.span_id()),
        Some(second.context.span_id())
    );
}

#[test]
fn an_ended_root_is_not_handed_back() {
    let harness = harness(settings());
    let root = harness.span("root");
    root.lock().end();
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    assert!(proxy::request_root_span(&harness.callback, &trace).is_none());
}

#[test]
fn the_root_is_stamped_without_a_transport() {
    let harness = harness(settings());
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    let facts = exception_facts(403);
    proxy::async_post_call_failure_hook(&harness.callback, Some(&facts), None, None, &trace);
    root.lock().end();
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
    proxy::async_post_call_failure_hook(
        &harness.callback,
        Some(&facts),
        None,
        Some(&parent),
        &trace,
    );
    parent.lock().end();
    let spans = harness.finished();
    assert_eq!(
        attribute(&spans[0], "error.type").as_deref(),
        Some("AuthenticationError")
    );
}

#[test]
fn the_failure_hook_keeps_the_traceback_on_the_anchored_root() {
    let harness = harness(settings());
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    let mut facts = exception_facts(500);
    facts.llm_provider = None;
    let traceback = "Traceback (most recent call last):\n";
    proxy::async_post_call_failure_hook(
        &harness.callback,
        Some(&facts),
        Some(traceback),
        None,
        &trace,
    );
    root.lock().end();
    let spans = harness.finished();
    assert_eq!(
        attribute(&spans[0], "litellm.provider.error.stack_trace").as_deref(),
        Some(traceback)
    );
}
