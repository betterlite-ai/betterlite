#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Status, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use crate::otel::emitter::error_stamping::{
    ErrorStamp, error_attributes, resolve_error, stamp_error,
};
use crate::otel::model::payloads::error::SpanError;

fn error(
    error_type: Option<&str>,
    message: Option<&str>,
    code: Option<&str>,
    stack_trace: Option<&str>,
    llm_provider: Option<&str>,
) -> SpanError {
    SpanError {
        error_type: error_type.map(str::to_owned),
        message: message.map(str::to_owned),
        code: code.map(str::to_owned),
        stack_trace: stack_trace.map(str::to_owned),
        llm_provider: llm_provider.map(str::to_owned),
    }
}

#[test]
fn an_error_with_no_type_and_no_message_resolves_to_nothing() {
    assert_eq!(resolve_error(&SpanError::default()), None);
    assert!(error_attributes(&SpanError::default()).is_empty());
}

#[test]
fn a_message_without_a_type_resolves_the_type_to_error() {
    let resolved = resolve_error(&error(None, Some("slow down"), None, None, None));
    assert_eq!(resolved, Some(("error".to_owned(), "slow down".to_owned())));
}

#[test]
fn a_type_without_a_message_resolves_the_message_to_the_type() {
    let resolved = resolve_error(&error(Some("RateLimitError"), None, None, None, None));
    assert_eq!(
        resolved,
        Some(("RateLimitError".to_owned(), "RateLimitError".to_owned()))
    );
}

#[test]
fn an_empty_type_is_used_as_given() {
    let resolved = resolve_error(&error(Some(""), Some("slow down"), None, None, None));
    assert_eq!(resolved, Some((String::new(), "slow down".to_owned())));
    let attributes = error_attributes(&error(Some(""), None, Some(""), None, None));
    assert_eq!(attributes.get("error.type"), Some(&Value::from("")));
    assert_eq!(
        attributes.get("litellm.provider.error.code"),
        Some(&Value::from(""))
    );
}

#[test]
fn error_attributes_carry_only_the_detail_keys_that_exist() {
    let attributes = error_attributes(&error(
        Some("RateLimitError"),
        Some("slow down"),
        Some("429"),
        None,
        Some("openai"),
    ));
    assert_eq!(
        attributes.keys().map(String::as_str).collect::<Vec<_>>(),
        vec![
            "error.message",
            "error.type",
            "litellm.provider.error.code",
            "litellm.provider.error.llm_provider"
        ]
    );
}

#[test]
fn stamp_error_writes_the_attributes_and_returns_the_resolved_pair() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("error-stamping");
    let mut span = tracer.start("call");
    let resolved = stamp_error(
        &mut span,
        &error(Some("RateLimitError"), Some("slow down"), None, None, None),
        ErrorStamp::FULL,
    );
    span.end();
    assert_eq!(
        resolved,
        Some(("RateLimitError".to_owned(), "slow down".to_owned()))
    );
    let finished = exporter.get_finished_spans().expect("spans");
    let exported_span = &finished[0];
    assert_eq!(exported_span.status, Status::error("slow down"));
    assert_eq!(exported_span.events.events.len(), 1);
    assert_eq!(exported_span.events.events[0].name, "exception");
}

#[test]
fn stamp_error_attributes_only_leaves_the_status_and_events_alone() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("error-stamping");
    let mut span = tracer.start("call");
    stamp_error(
        &mut span,
        &error(Some("RateLimitError"), Some("slow down"), None, None, None),
        ErrorStamp::ATTRIBUTES_ONLY,
    );
    span.end();
    let finished = exporter.get_finished_spans().expect("spans");
    let exported_span = &finished[0];
    let attributes: BTreeMap<String, Value> = exported_span
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    assert_eq!(
        attributes.get("error.type"),
        Some(&Value::from("RateLimitError"))
    );
    assert_eq!(exported_span.status, Status::Unset);
    assert!(exported_span.events.events.is_empty());
}

#[test]
fn stamping_nothing_returns_none_and_writes_nothing() {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("error-stamping");
    let mut span = tracer.start("call");
    assert_eq!(
        stamp_error(&mut span, &SpanError::default(), ErrorStamp::FULL),
        None
    );
    span.end();
    let finished = exporter.get_finished_spans().expect("spans");
    assert!(finished[0].attributes.is_empty());
    assert_eq!(finished[0].status, Status::Unset);
}
