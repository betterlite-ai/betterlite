#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Context;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    Span as _, SpanBuilder, SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId,
    TraceState, Tracer as _, TracerProvider as _,
};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use crate::otel::plumbing::context::baggage_context::set_request_baggage;
use crate::otel::plumbing::context::context_slots::{AnchoredSpan, RequestTraceContext};

fn span_context(trace_id: u128, span_id: u64) -> SpanContext {
    SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

fn anchored(trace_id: u128, span_id: u64, http_route: Option<&str>) -> AnchoredSpan {
    AnchoredSpan {
        context: span_context(trace_id, span_id),
        http_route: http_route.map(str::to_owned),
    }
}

fn exporter_and_provider() -> (InMemorySpanExporter, SdkTracerProvider) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    (exporter, provider)
}

#[test]
fn a_child_started_on_the_anchored_context_parents_to_the_root_span() {
    let (exporter, provider) = exporter_and_provider();
    let tracer = provider.tracer("context-slots");
    let mut root = tracer.start("root");
    let root_context = root.span_context().clone();
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: root_context.clone(),
        http_route: Some("/v1/chat/completions".to_owned()),
    });
    let parent = trace.resolve_request_span_context();
    let mut child = tracer.build_with_context(SpanBuilder::from_name("child"), &parent);
    child.end();
    root.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let exported_child = spans
        .iter()
        .find(|span| span.name == "child")
        .expect("child");
    assert_eq!(
        exported_child.span_context.trace_id(),
        root_context.trace_id()
    );
    assert_eq!(exported_child.parent_span_id, root_context.span_id());
}

#[test]
fn the_anchor_wins_over_an_ambient_span() {
    let ambient = Context::new().with_remote_span_context(span_context(1, 2));
    let _guard = ambient.attach();
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(9, 10, None));
    let parent = trace.resolve_request_span_context();
    assert_eq!(parent.span().span_context().trace_id(), TraceId::from(9));
}

#[test]
fn the_latest_anchor_replaces_the_earlier_one() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(1, 2, None));
    trace.anchor_request_root(anchored(3, 4, None));
    assert_eq!(
        trace
            .resolve_request_span_context()
            .span()
            .span_context()
            .trace_id(),
        TraceId::from(3)
    );
}

#[test]
fn an_invalid_span_never_replaces_a_good_anchor() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(1, 2, None));
    trace.anchor_request_root(AnchoredSpan {
        context: SpanContext::NONE,
        http_route: None,
    });
    assert_eq!(
        trace
            .resolve_request_span_context()
            .span()
            .span_context()
            .trace_id(),
        TraceId::from(1)
    );
}

#[test]
fn without_an_anchor_the_ambient_context_is_used() {
    let _ambient = Context::new()
        .with_remote_span_context(span_context(1, 2))
        .attach();
    let trace = RequestTraceContext::default();
    let context = trace.resolve_request_span_context();
    assert_eq!(context.span().span_context().trace_id(), TraceId::from(1));
}

#[test]
fn the_anchored_context_keeps_the_ambient_baggage() {
    let _ambient = set_request_baggage(
        &BTreeMap::from([("litellm.team.id".to_owned(), "team-1".to_owned())]),
        None,
    )
    .attach();
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(9, 10, None));
    let parent = trace.resolve_request_span_context();
    assert_eq!(parent.span().span_context().trace_id(), TraceId::from(9));
    assert_eq!(
        parent
            .baggage()
            .get("litellm.team.id")
            .map(|value| value.as_str().to_owned()),
        Some("team-1".to_owned())
    );
}

#[test]
fn the_anchor_carries_the_http_route_the_root_span_reported() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(1, 2, Some("/v1/chat/completions")));
    let root = trace.root.as_ref().expect("root");
    assert_eq!(root.http_route.as_deref(), Some("/v1/chat/completions"));
}

#[test]
fn an_anchored_root_with_no_route_reports_none() {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(anchored(1, 2, None));
    let root = trace.root.as_ref().expect("root");
    assert_eq!(root.http_route, None);
}
