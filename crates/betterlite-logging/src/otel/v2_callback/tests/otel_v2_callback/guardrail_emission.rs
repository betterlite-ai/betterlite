#![cfg(test)]

use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::trace::SpanId;
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor, SpanData,
};

use betterlite_logging::otel::plumbing::context::context_slots::RequestTraceContext;
use betterlite_logging::otel::v2_callback::callback::V2Callback;
use betterlite_logging::otel::v2_callback::guardrail::emit_guardrail_span;
use betterlite_logging::otel::v2_callback::proxy;
use betterlite_logging::otel::v2_callback::registered;
use betterlite_logging::otel::v2_callback::registry::V2Registry;

use super::support::{attribute, construction_parts, guardrail_entry, harness, settings};

fn provider(spans: &InMemorySpanExporter) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(SimpleSpanProcessor::new(spans.clone()))
        .build()
}

#[test]
fn the_module_entry_emits_one_span_through_the_registered_callback() {
    let first_spans = InMemorySpanExporter::default();
    let second_spans = InMemorySpanExporter::default();
    let registry = V2Registry::default();
    let _first = V2Callback::new(
        construction_parts(settings(), &first_spans, Some(provider(&first_spans))),
        &registry,
    )
    .expect("first callback");
    let _second = V2Callback::new(
        construction_parts(settings(), &second_spans, Some(provider(&second_spans))),
        &registry,
    )
    .expect("second callback");
    let entry = guardrail_entry("pii");
    let trace = RequestTraceContext::default();
    registered::emit_guardrail_span(&registry, &entry, &trace);
    let first = first_spans.get_finished_spans().expect("first spans");
    let second = second_spans.get_finished_spans().expect("second spans");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name.as_ref(), "execute_guardrail pii");
    assert!(second.is_empty());
}

#[test]
fn without_a_registered_callback_the_entry_is_dropped() {
    let registry = V2Registry::default();
    let entry = guardrail_entry("pii");
    let trace = RequestTraceContext::default();
    registered::emit_guardrail_span(&registry, &entry, &trace);
    assert!(registry.registered_v2_logger().is_none());
}

#[test]
fn the_instance_method_parents_under_the_anchored_root() {
    let harness = harness(settings());
    let root = harness.span("root");
    let mut trace = RequestTraceContext::default();
    proxy::mark_request_root_span(&harness.callback, &root, None, &mut trace);
    emit_guardrail_span(&harness.callback, &guardrail_entry("pii"), &trace);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, root.context.span_id());
    assert_eq!(
        attribute(&spans[0], "litellm.guardrail.name").as_deref(),
        Some("pii")
    );
    assert_eq!(
        attribute(&spans[0], "litellm.guardrail.status").as_deref(),
        Some("success")
    );
    assert_entry_times(&spans[0]);
}

fn assert_entry_times(span: &SpanData) {
    assert_eq!(
        span.start_time
            .duration_since(UNIX_EPOCH)
            .expect("start after the epoch")
            .as_secs(),
        1_704_067_200
    );
    assert_eq!(
        span.end_time
            .duration_since(UNIX_EPOCH)
            .expect("end after the epoch")
            .as_millis(),
        1_704_067_201_500
    );
    assert_eq!(
        span.end_time
            .duration_since(span.start_time)
            .expect("end after start"),
        Duration::from_millis(1_500)
    );
}

#[test]
fn the_instance_method_starts_a_root_without_an_anchor() {
    let harness = harness(settings());
    let trace = RequestTraceContext::default();
    emit_guardrail_span(&harness.callback, &guardrail_entry("pii"), &trace);
    let spans = harness.finished();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
}
