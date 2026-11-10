#![cfg(test)]

use std::time::UNIX_EPOCH;

use opentelemetry::Value;

use super::grounding::end_grounding_span;
use super::support::child;
use super::support::{
    attribute, attribute_json, generation, prompt_value, provider, sdk_tracer, span, spans,
};

#[test]
fn grounding_span_creates_and_ends_a_child_span() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    let value = prompt_value(r#"{"uri": "https://example.com"}"#);
    end_grounding_span(&observations, "sources", &value);
    let finished = spans(&exporter);
    assert_eq!(finished.len(), 1);
    let data = finished.first().expect("a span");
    assert_eq!(data.name, "sources");
    assert_eq!(data.parent_span_id, span("0102030405060708"));
    assert_eq!(
        attribute(data, "langfuse.observation.type"),
        Some(Value::from("span"))
    );
    assert_eq!(
        attribute_json(data, "langfuse.observation.input"),
        Some(prompt_value(r#"{"uri": "https://example.com"}"#))
    );
    assert_ne!(data.end_time, UNIX_EPOCH);
}

#[test]
fn grounding_span_writes_a_text_value_as_is() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    end_grounding_span(
        &observations,
        "vertex_ai_grounding_metadata",
        &prompt_value(r#""grounded""#),
    );
    let data = spans(&exporter).first().cloned().expect("a span");
    assert_eq!(
        attribute(&data, "langfuse.observation.input"),
        Some(Value::from("grounded"))
    );
}

#[test]
fn grounding_span_omits_a_null_input() {
    let (provider, exporter) = provider();
    let tracer = sdk_tracer(&provider);
    let parent = generation(&tracer);
    let observations = child(&tracer, &parent);
    end_grounding_span(&observations, "sources", &prompt_value("null"));
    let data = spans(&exporter).first().cloned().expect("a span");
    assert!(attribute(&data, "langfuse.observation.input").is_none());
}
