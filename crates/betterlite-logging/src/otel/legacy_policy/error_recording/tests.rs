#![cfg(test)]

use std::collections::BTreeMap;

use opentelemetry::Value;
use opentelemetry::trace::{Event, Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use super::{
    ERROR_CODE, ERROR_LLM_PROVIDER, ERROR_STACK_TRACE, RecordedException, record_exception_on_span,
};
use crate::otel::model::utils::coercion::OtelModelValue;

fn payload_map(text: &str) -> BTreeMap<String, OtelModelValue> {
    serde_json::from_str(text).expect("payload")
}

fn record(
    exception: Option<&RecordedException>,
    payload: &BTreeMap<String, OtelModelValue>,
) -> (Vec<(String, Value)>, Vec<Event>) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("legacy-policy-tests").start("test");
    record_exception_on_span(&mut span, exception, payload);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let first = spans.first().expect("one span");
    let attributes = first
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    (attributes, first.events.events.clone())
}

fn exception() -> RecordedException {
    RecordedException {
        type_name: "ValueError".to_owned(),
        message: "boom".to_owned(),
        stacktrace: "Traceback (most recent call last)".to_owned(),
    }
}

#[test]
fn a_present_exception_becomes_an_exception_event() {
    let payload = payload_map("{}");
    let recorded = exception();
    let (attributes, events) = record(Some(&recorded), &payload);
    assert!(attributes.is_empty());
    let event = events.first().expect("one event");
    assert_eq!(event.name.as_ref(), "exception");
    let pairs: Vec<(String, Value)> = event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("exception.type".to_owned(), Value::from("ValueError")),
            ("exception.message".to_owned(), Value::from("boom")),
            (
                "exception.stacktrace".to_owned(),
                Value::from("Traceback (most recent call last)")
            ),
            ("exception.escaped".to_owned(), Value::from("False")),
        ]
    );
}

#[test]
fn an_exception_without_a_stacktrace_uses_the_formatted_exception() {
    let payload = payload_map("{}");
    let recorded = RecordedException {
        type_name: "ValueError".to_owned(),
        message: "boom".to_owned(),
        stacktrace: String::new(),
    };
    let (_attributes, events) = record(Some(&recorded), &payload);
    let event = events.first().expect("one event");
    let pairs: Vec<(String, Value)> = event
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("exception.type".to_owned(), Value::from("ValueError")),
            ("exception.message".to_owned(), Value::from("boom")),
            (
                "exception.stacktrace".to_owned(),
                Value::from("ValueError: boom\n")
            ),
            ("exception.escaped".to_owned(), Value::from("False")),
        ]
    );
}

#[test]
fn no_exception_writes_no_event() {
    let payload = payload_map("{}");
    let (_attributes, events) = record(None, &payload);
    assert!(events.is_empty());
}

#[test]
fn a_missing_error_information_falls_back_to_error_str() {
    let payload = payload_map(r#"{"error_str": "upstream failed"}"#);
    let (attributes, _events) = record(None, &payload);
    assert_eq!(
        attributes,
        vec![("error.message".to_owned(), Value::from("upstream failed"))]
    );
}

#[test]
fn a_null_error_information_falls_back_to_error_str() {
    let payload = payload_map(r#"{"error_information": null, "error_str": "upstream failed"}"#);
    let (attributes, _events) = record(None, &payload);
    assert_eq!(
        attributes,
        vec![("error.message".to_owned(), Value::from("upstream failed"))]
    );
}

#[test]
fn an_empty_error_information_skips_the_error_str_fallback() {
    let payload = payload_map(r#"{"error_information": {}, "error_str": "upstream failed"}"#);
    let (attributes, _events) = record(None, &payload);
    assert!(attributes.is_empty());
}

#[test]
fn a_numeric_code_also_sets_the_http_status() {
    let payload = payload_map(
        r#"{"error_information": {
            "error_code": "429",
            "error_class": "RateLimitError",
            "error_message": "slow down",
            "llm_provider": "openai",
            "traceback": "tb"
        }}"#,
    );
    let (attributes, _events) = record(None, &payload);
    assert_eq!(
        attributes,
        vec![
            (ERROR_CODE.to_owned(), Value::from("429")),
            ("http.response.status_code".to_owned(), Value::I64(429)),
            ("error.type".to_owned(), Value::from("RateLimitError")),
            ("error.message".to_owned(), Value::from("slow down")),
            (ERROR_LLM_PROVIDER.to_owned(), Value::from("openai")),
            (ERROR_STACK_TRACE.to_owned(), Value::from("tb")),
        ]
    );
}

#[test]
fn a_text_code_keeps_only_the_error_code() {
    let payload = payload_map(r#"{"error_information": {"error_code": "bad_request"}}"#);
    let (attributes, _events) = record(None, &payload);
    assert_eq!(
        attributes,
        vec![(ERROR_CODE.to_owned(), Value::from("bad_request"))]
    );
}

#[test]
fn empty_error_fields_are_skipped() {
    let payload = payload_map(
        r#"{"error_information": {
            "error_code": "",
            "error_class": "",
            "error_message": "",
            "llm_provider": "",
            "traceback": ""
        }}"#,
    );
    let (attributes, _events) = record(None, &payload);
    assert!(attributes.is_empty());
}

#[test]
fn an_empty_error_str_writes_nothing() {
    let payload = payload_map(r#"{"error_str": ""}"#);
    let (attributes, _events) = record(None, &payload);
    assert!(attributes.is_empty());
}
