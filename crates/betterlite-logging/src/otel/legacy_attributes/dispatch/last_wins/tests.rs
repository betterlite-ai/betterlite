#![cfg(test)]

use std::time::{Duration, SystemTime};

use opentelemetry::trace::noop::NoopTracer;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, Status, TraceFlags, TraceId, TraceState, Tracer as _,
    TracerProvider as _,
};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan, SpanData,
};

use super::LastWins;

fn finished<F>(write: F) -> SpanData
where
    F: FnOnce(&mut LastWins<'_, SdkSpan>),
{
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3856-last-wins").start("case");
    LastWins::write(&mut span, write);
    span.end();
    exporter
        .get_finished_spans()
        .expect("spans")
        .into_iter()
        .next()
        .expect("one span")
}

#[test]
fn span_calls_other_than_attributes_reach_the_inner_span() {
    let linked = SpanContext::new(
        TraceId::from(7_u128),
        SpanId::from(9_u64),
        TraceFlags::SAMPLED,
        false,
        TraceState::default(),
    );
    let ended_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let data = finished(|buffered| {
        buffered.update_name("renamed");
        buffered.set_status(Status::error("boom"));
        buffered.add_link(
            linked.clone(),
            vec![KeyValue::new("link.key", "link.value")],
        );
        buffered.add_event("seen", vec![KeyValue::new("event.key", 1_i64)]);
        buffered.end_with_timestamp(ended_at);
    });
    assert_eq!(data.name, "renamed");
    assert_eq!(data.status, Status::error("boom"));
    assert_eq!(data.links.links.len(), 1);
    assert_eq!(data.links.links[0].span_context, linked);
    assert_eq!(
        data.links.links[0].attributes,
        vec![KeyValue::new("link.key", "link.value")]
    );
    assert_eq!(data.events.events.len(), 1);
    assert_eq!(data.events.events[0].name, "seen");
    assert_eq!(data.end_time, ended_at);
}

#[test]
fn the_wrapper_reports_the_inner_span_context_and_recording_state() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .build();
    let mut recording = provider.tracer("b3856-last-wins").start("case");
    let expected = recording.span_context().clone();
    let mut seen = None;
    LastWins::write(&mut recording, |buffered| {
        seen = Some((buffered.is_recording(), buffered.span_context().clone()));
    });
    assert_eq!(seen, Some((true, expected)));

    let mut silent = NoopTracer::new().start("case");
    let mut silent_seen = None;
    LastWins::write(&mut silent, |buffered| {
        silent_seen = Some(buffered.is_recording());
    });
    assert_eq!(silent_seen, Some(false));
}

#[test]
fn a_repeated_key_keeps_its_first_position_and_last_value() {
    let data = finished(|buffered| {
        buffered.set_attribute(KeyValue::new("first", "a"));
        buffered.set_attribute(KeyValue::new("second", "b"));
        buffered.set_attribute(KeyValue::new("first", "c"));
    });
    assert_eq!(
        data.attributes,
        vec![
            KeyValue::new("first", Value::from("c")),
            KeyValue::new("second", Value::from("b")),
        ]
    );
}
