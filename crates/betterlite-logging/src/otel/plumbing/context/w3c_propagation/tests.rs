#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use opentelemetry::Context;
use opentelemetry::KeyValue;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};

use crate::otel::plumbing::context::context_slots::{AnchoredSpan, RequestTraceContext};
use crate::otel::plumbing::context::w3c_propagation::{
    extract_traceparent, inject_trace_context, traceparent_from_header,
};

const CLIENT_TRACE: &str = "0af7651916cd43dd8448eb211c80319c";
const CLIENT_SPAN: &str = "b7ad6b7169203331";
const CLIENT_TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";

fn span_context(trace_id: u128, span_id: u64) -> SpanContext {
    SpanContext::new(
        TraceId::from(trace_id),
        SpanId::from(span_id),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    )
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn trace_with_root(trace_id: u128) -> RequestTraceContext {
    let mut trace = RequestTraceContext::default();
    trace.anchor_request_root(AnchoredSpan {
        context: span_context(trace_id, 42),
        http_route: None,
    });
    trace
}

#[test]
fn extract_traceparent_reads_mixed_case_headers() {
    let extracted =
        extract_traceparent(&headers(&[("TraceParent", CLIENT_TRACEPARENT)])).expect("context");
    assert_eq!(
        extracted.span().span_context().trace_id().to_string(),
        CLIENT_TRACE
    );
    assert_eq!(
        extracted.span().span_context().span_id().to_string(),
        CLIENT_SPAN
    );
}

#[test]
fn extract_traceparent_returns_none_without_the_header() {
    assert!(extract_traceparent(&headers(&[("tracestate", "vendor=value")])).is_none());
    assert!(extract_traceparent(&BTreeMap::new()).is_none());
}

#[test]
fn extract_traceparent_returns_no_span_for_a_malformed_value() {
    let _ambient = Context::new()
        .with_remote_span_context(span_context(1, 2))
        .attach();
    let context = extract_traceparent(&headers(&[("traceparent", "00-not-a-traceparent")]));
    assert!(!context.expect("context").span().span_context().is_valid());
}

#[test]
fn inject_trace_context_adds_lowercase_trace_headers() {
    let parent = SpanContext::new(
        TraceId::from(7_u128),
        SpanId::from(8_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::from_str("vendor=value").expect("tracestate"),
    );
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &headers(&[("content-type", "application/json")]),
        Some(&parent),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-00000000000000000000000000000007-0000000000000008-01")
    );
    assert_eq!(
        injected.get("tracestate").map(String::as_str),
        Some("vendor=value")
    );
    assert_eq!(
        injected.get("content-type").map(String::as_str),
        Some("application/json")
    );
}

#[test]
fn unknown_trace_flag_bits_are_zeroed_on_injection() {
    let parent = SpanContext::new(
        TraceId::from(7_u128),
        SpanId::from(8_u64),
        TraceFlags::new(0x03),
        false,
        TraceState::default(),
    );
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &BTreeMap::new(),
        Some(&parent),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-00000000000000000000000000000007-0000000000000008-01")
    );
}

#[test]
fn inject_trace_context_does_not_write_an_empty_tracestate() {
    let parent = span_context(7, 8);
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &BTreeMap::new(),
        Some(&parent),
    );
    assert!(injected.contains_key("traceparent"));
    assert!(!injected.contains_key("tracestate"));
}

#[test]
fn inject_trace_context_lowercases_the_trace_headers_it_writes() {
    let parent = span_context(7, 8);
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &headers(&[
            ("Traceparent", CLIENT_TRACEPARENT),
            ("TRACESTATE", "vendor=client"),
            ("x-request-id", "req-1"),
        ]),
        Some(&parent),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
    );
    assert_eq!(
        injected.get("tracestate").map(String::as_str),
        Some("vendor=client")
    );
    assert!(!injected.contains_key("Traceparent"));
    assert!(!injected.contains_key("TRACESTATE"));
    assert_eq!(
        injected.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
}

#[test]
fn an_upstream_traceparent_of_another_trace_is_forwarded() {
    let parent = span_context(7, 8);
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &headers(&[("traceparent", CLIENT_TRACEPARENT)]),
        Some(&parent),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01")
    );
}

#[test]
fn an_upstream_traceparent_of_the_same_trace_is_replaced() {
    let client = format!("00-{CLIENT_TRACE}-0000000000000001-01");
    let parent = SpanContext::new(
        TraceId::from_hex(CLIENT_TRACE).expect("trace id"),
        SpanId::from(8_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );
    let injected = inject_trace_context(
        &RequestTraceContext::default(),
        &headers(&[("traceparent", client.as_str())]),
        Some(&parent),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-0af7651916cd43dd8448eb211c80319c-0000000000000008-01")
    );
}

#[test]
fn an_invalid_explicit_parent_falls_back_to_the_anchored_root() {
    let injected = inject_trace_context(
        &trace_with_root(7),
        &BTreeMap::new(),
        Some(&SpanContext::NONE),
    );
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-00000000000000000000000000000007-000000000000002a-01")
    );
}

#[test]
fn inject_trace_context_falls_back_to_the_anchored_root() {
    let injected = inject_trace_context(&trace_with_root(7), &BTreeMap::new(), None);
    assert_eq!(
        injected.get("traceparent").map(String::as_str),
        Some("00-00000000000000000000000000000007-000000000000002a-01")
    );
}

#[test]
fn inject_trace_context_leaves_the_headers_alone_without_a_span() {
    let headers = headers(&[
        ("traceparent", CLIENT_TRACEPARENT),
        ("x-request-id", "req-1"),
    ]);
    let injected = inject_trace_context(&RequestTraceContext::default(), &headers, None);
    assert_eq!(injected, headers);
}

#[test]
fn inject_trace_context_never_injects_baggage() {
    let ambient = Context::new()
        .with_baggage([KeyValue::new("litellm.team.id", "team-1")])
        .with_remote_span_context(span_context(7, 8));
    let _guard = ambient.attach();
    let injected = inject_trace_context(&RequestTraceContext::default(), &BTreeMap::new(), None);
    assert!(injected.contains_key("traceparent"));
    assert!(!injected.contains_key("baggage"));
}

#[test]
fn traceparent_from_header_extracts_the_context() {
    let headers = BTreeMap::from([(
        "traceparent".to_owned(),
        Some(CLIENT_TRACEPARENT.to_owned()),
    )]);
    let context = traceparent_from_header(Some(&headers)).expect("context");
    assert_eq!(
        context.span().span_context().trace_id().to_string(),
        CLIENT_TRACE
    );
}

#[test]
fn traceparent_from_header_requires_the_exact_lowercase_key() {
    let headers = BTreeMap::from([(
        "Traceparent".to_owned(),
        Some(CLIENT_TRACEPARENT.to_owned()),
    )]);
    assert!(traceparent_from_header(Some(&headers)).is_none());
    assert!(traceparent_from_header(None).is_none());
    assert!(traceparent_from_header(Some(&BTreeMap::new())).is_none());
}

#[test]
fn traceparent_from_header_returns_no_span_for_a_malformed_value() {
    let _ambient = Context::new()
        .with_remote_span_context(span_context(1, 2))
        .attach();
    let headers = BTreeMap::from([("traceparent".to_owned(), Some("garbage".to_owned()))]);
    let context = traceparent_from_header(Some(&headers)).expect("context");
    assert!(!context.span().span_context().is_valid());
}

#[test]
fn traceparent_from_header_skips_a_null_value() {
    let headers = BTreeMap::from([("traceparent".to_owned(), None)]);
    assert!(traceparent_from_header(Some(&headers)).is_none());
}
