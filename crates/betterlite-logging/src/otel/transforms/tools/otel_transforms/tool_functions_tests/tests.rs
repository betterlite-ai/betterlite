#![cfg(test)]

use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::transforms::tools::{set_tools_attributes, tool_calls_kv_pair};

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn exported_attributes(exporter: &InMemorySpanExporter) -> Vec<(String, Value)> {
    let spans = exporter.get_finished_spans().expect("spans");
    let span = spans.first().expect("one span");
    span.attributes
        .iter()
        .map(|pair: &KeyValue| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn span_with_tools(tools: &[OtelModelValue]) -> Vec<(String, Value)> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("b3845").start("tools");
    set_tools_attributes(&mut span, tools);
    span.end();
    exported_attributes(&exporter)
}

#[test]
fn set_tools_attributes_writes_every_field_of_every_usable_tool() {
    let tools = [
        value(
            r#"{"type": "function", "function": {"name": "get_weather", "description": "Weather", "parameters": {"type": "object"}}}"#,
        ),
        value(r#"{"type": "function", "function": {"name": "no_params"}}"#),
        value(r#"{"type": "function", "function": {}}"#),
        value(r#"{"type": "function"}"#),
    ];
    assert_eq!(
        span_with_tools(&tools),
        vec![
            (
                "llm.request.functions.0.name".to_owned(),
                Value::from("get_weather")
            ),
            (
                "llm.request.functions.0.description".to_owned(),
                Value::from("Weather")
            ),
            (
                "llm.request.functions.0.parameters".to_owned(),
                Value::from(r#"{"type":"object"}"#)
            ),
            (
                "llm.request.functions.1.name".to_owned(),
                Value::from("no_params")
            ),
            (
                "llm.request.functions.1.description".to_owned(),
                Value::from("")
            ),
            (
                "llm.request.functions.1.parameters".to_owned(),
                Value::from("null")
            ),
        ]
    );
}

#[test]
fn tool_calls_emit_only_name_and_arguments_when_set() {
    let tool_calls = [
        value(r#"{"function": {"name": "f", "arguments": "{\"a\":1}"}}"#),
        value(r#"{"function": {"name": "", "arguments": ""}}"#),
        value(r#"{"function": {}}"#),
        value(r#"{"function": null}"#),
    ];
    let pairs = tool_calls_kv_pair(&tool_calls);
    let rendered: Vec<(String, String)> = pairs
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                value.as_str().map(str::to_owned).unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        rendered,
        vec![
            (
                "gen_ai.completion.0.function_call.arguments".to_owned(),
                r#"{"a":1}"#.to_owned()
            ),
            (
                "gen_ai.completion.0.function_call.name".to_owned(),
                "f".to_owned()
            ),
        ]
    );
}
