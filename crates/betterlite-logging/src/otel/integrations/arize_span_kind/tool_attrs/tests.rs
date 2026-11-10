#![cfg(test)]

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::collections::BTreeMap;

use crate::otel::integrations::arize_span_kind::set_tool_attributes;
use crate::otel::model::utils::coercion::OtelModelValue;

fn tools(json: &str) -> Vec<OtelModelValue> {
    serde_json::from_str(json).expect("tools")
}

fn recorded(
    optional_tools: Option<&[OtelModelValue]>,
    metadata_tools: Option<&[OtelModelValue]>,
) -> BTreeMap<String, Value> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("tool-attrs").start("test");
    set_tool_attributes(&mut span, optional_tools, metadata_tools);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    spans
        .first()
        .expect("one span")
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect()
}

fn json_text(value: &Value) -> OtelModelValue {
    let Value::String(text) = value else {
        panic!("attribute is not text");
    };
    serde_json::from_str(text.as_str()).expect("json attribute")
}

#[test]
fn tool_attrs_writes_the_optional_tool_fields() {
    let declared = tools(
        r#"[{"type": "function", "function": {"name": "get_weather", "description": "Weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}}]"#,
    );
    let attributes = recorded(Some(&declared), None);
    assert_eq!(
        attributes.get("llm.tools.0.name"),
        Some(&Value::from("get_weather"))
    );
    assert_eq!(
        attributes.get("llm.tools.0.description"),
        Some(&Value::from("Weather"))
    );
    let expected: OtelModelValue =
        serde_json::from_str(r#"{"type": "object", "properties": {"city": {"type": "string"}}}"#)
            .expect("expected");
    let actual = json_text(
        attributes
            .get("llm.tools.0.parameters")
            .expect("parameters"),
    );
    assert_eq!(
        serde_json::to_string(&actual).expect("json"),
        serde_json::to_string(&expected).expect("json")
    );
}

#[test]
fn tool_attrs_skips_non_dict_tools_and_keeps_the_index() {
    let declared = tools(
        r#"[{"function": {"name": "first"}}, "not a tool", {"function": {"name": "third"}}]"#,
    );
    let attributes = recorded(Some(&declared), None);
    assert_eq!(
        attributes.get("llm.tools.0.name"),
        Some(&Value::from("first"))
    );
    assert_eq!(
        attributes.get("llm.tools.2.name"),
        Some(&Value::from("third"))
    );
    assert!(!attributes.contains_key("llm.tools.1.name"));
}

#[test]
fn tool_attrs_skips_a_tool_without_a_function() {
    let declared = tools(r#"[{"type": "function"}, {"function": {"name": "second"}}]"#);
    let attributes = recorded(Some(&declared), None);
    assert!(!attributes.contains_key("llm.tools.0.name"));
    assert_eq!(
        attributes.get("llm.tools.1.name"),
        Some(&Value::from("second"))
    );
}

#[test]
fn tool_attrs_writes_the_metadata_tools() {
    let metadata =
        tools(r#"[{"name": "meta_tool", "description": "Meta"}, "not a tool", {"name": "third"}]"#);
    let attributes = recorded(None, Some(&metadata));
    assert_eq!(
        attributes.get("llm.invocation_parameters.tools.0.name"),
        Some(&Value::from("meta_tool"))
    );
    assert_eq!(
        attributes.get("llm.invocation_parameters.tools.0.description"),
        Some(&Value::from("Meta"))
    );
    assert_eq!(
        attributes.get("llm.invocation_parameters.tools.2.name"),
        Some(&Value::from("third"))
    );
    assert!(!attributes.contains_key("llm.invocation_parameters.tools.1.name"));
}

#[test]
fn tool_attrs_omits_null_parameters_and_keeps_an_empty_description() {
    let declared =
        tools(r#"[{"function": {"name": "no_params", "description": "", "parameters": null}}]"#);
    let attributes = recorded(Some(&declared), None);
    assert_eq!(
        attributes.get("llm.tools.0.name"),
        Some(&Value::from("no_params"))
    );
    assert_eq!(
        attributes.get("llm.tools.0.description"),
        Some(&Value::from(""))
    );
    assert!(!attributes.contains_key("llm.tools.0.parameters"));
}

#[test]
fn tool_attrs_writes_nothing_without_tools() {
    assert!(recorded(None, None).is_empty());
    let empty: Vec<OtelModelValue> = Vec::new();
    assert!(recorded(Some(&empty), Some(&empty)).is_empty());
}
