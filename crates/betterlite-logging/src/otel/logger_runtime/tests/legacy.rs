#![cfg(test)]

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use chrono::{TimeZone as _, Utc};
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, SpanId, SpanKind, Status, TraceId, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use rstest::rstest;

use crate::otel::legacy_policy::span_time::SpanTime;
use crate::otel::logger_runtime::legacy::{
    set_proxy_request_route, set_response_status_code, start_proxy_request_span,
};
use crate::otel::v2_callback::live_span::LiveSpan;
use crate::otel::vocabulary::constants::attribute_keys::http::{
    RESPONSE_STATUS_CODE, ROUTE, URL_PATH,
};
use crate::otel::vocabulary::span_names::PROXY_REQUEST_SPAN_NAME;

use super::support::{attribute, provider_exporting};

const NOW_NS: i64 = 1_700_000_000_000_000_000;

fn header_map(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

fn proxy_span(
    sink: &InMemorySpanExporter,
    start_time: SpanTime,
    now_ns: i64,
    headers: Option<&BTreeMap<String, Option<String>>>,
) -> (SdkTracerProvider, LiveSpan) {
    let provider = provider_exporting(sink);
    let tracer = provider.tracer("legacy");
    let span = start_proxy_request_span(&tracer, start_time, now_ns, headers);
    (provider, span)
}

#[test]
fn legacy_parent_remote_context() {
    let sink = InMemorySpanExporter::default();
    let headers = header_map(&[(
        "traceparent",
        Some("00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"),
    )]);
    let (_provider, span) = proxy_span(&sink, SpanTime::Missing, NOW_NS, Some(&headers));
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    let data = &spans[0];
    assert_eq!(
        data.span_context.trace_id(),
        TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").expect("trace id")
    );
    assert_eq!(
        data.parent_span_id,
        SpanId::from_hex("b7ad6b7169203331").expect("parent id")
    );
    assert!(data.parent_span_is_remote);
}

#[rstest]
#[case::datetime(
    SpanTime::DateTime(Utc.timestamp_opt(1_704_067_200, 0).single().expect("timestamp")),
    1_704_067_200
)]
#[case::seconds(SpanTime::Seconds(1_704_067_200.0), 1_704_067_200)]
#[case::missing(SpanTime::Missing, 5)]
fn legacy_parent_start_time(#[case] start_time: SpanTime, #[case] expected_seconds: u64) {
    let sink = InMemorySpanExporter::default();
    let (_provider, span) = proxy_span(&sink, start_time, 5_000_000_000, None);
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    let data = &spans[0];
    assert_eq!(
        Some(data.start_time),
        UNIX_EPOCH.checked_add(Duration::from_secs(expected_seconds))
    );
    assert_eq!(data.span_kind, SpanKind::Server);
    assert_eq!(data.name.as_ref(), PROXY_REQUEST_SPAN_NAME);
    assert_eq!(data.parent_span_id, SpanId::INVALID);
}

#[test]
fn legacy_route_missing_values() {
    let sink = InMemorySpanExporter::default();
    let (_provider, span) = proxy_span(&sink, SpanTime::Missing, NOW_NS, None);
    set_proxy_request_route(None, Some("/ignored"), Some("/ignored"));
    set_proxy_request_route(Some(&span), None, Some(""));
    assert!(!span.has_attribute(URL_PATH));
    assert!(!span.has_attribute(ROUTE));
    set_proxy_request_route(Some(&span), Some("/v1/chat/completions"), Some("/v1/chat"));
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    let data = &spans[0];
    assert_eq!(
        attribute(data, URL_PATH),
        Some(&Value::from("/v1/chat/completions"))
    );
    assert_eq!(attribute(data, ROUTE), Some(&Value::from("/v1/chat")));
}

#[test]
fn legacy_setters_leave_the_span_open() {
    let sink = InMemorySpanExporter::default();
    let (_provider, span) = proxy_span(&sink, SpanTime::Missing, NOW_NS, None);
    set_proxy_request_route(Some(&span), Some("/v1/chat"), Some("/chat"));
    assert!(span.lock().is_recording());
    set_response_status_code(Some(&span), Some(201));
    assert!(span.lock().is_recording());
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    let data = &spans[0];
    assert_eq!(attribute(data, URL_PATH), Some(&Value::from("/v1/chat")));
    assert_eq!(attribute(data, ROUTE), Some(&Value::from("/chat")));
    assert_eq!(
        attribute(data, RESPONSE_STATUS_CODE),
        Some(&Value::I64(201))
    );
    assert_eq!(data.status, Status::Unset);
}

#[test]
fn legacy_status_zero() {
    let sink = InMemorySpanExporter::default();
    let (_provider, span) = proxy_span(&sink, SpanTime::Missing, NOW_NS, None);
    set_response_status_code(None, Some(500));
    set_response_status_code(Some(&span), Some(0));
    assert!(span.has_attribute(RESPONSE_STATUS_CODE));
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    assert_eq!(
        attribute(&spans[0], RESPONSE_STATUS_CODE),
        Some(&Value::I64(0))
    );
}

#[test]
fn legacy_status_none_writes_nothing() {
    let sink = InMemorySpanExporter::default();
    let (_provider, span) = proxy_span(&sink, SpanTime::Missing, NOW_NS, None);
    set_response_status_code(Some(&span), None);
    assert!(!span.has_attribute(RESPONSE_STATUS_CODE));
    span.lock().end();
    let spans = sink.get_finished_spans().expect("exported spans");
    assert_eq!(spans.len(), 1);
    assert_eq!(attribute(&spans[0], RESPONSE_STATUS_CODE), None);
}
