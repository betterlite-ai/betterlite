#![cfg(test)]

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span};
use rstest::rstest;

use super::set_response_attributes;
use crate::otel::model::utils::coercion::OtelModelValue;

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn response_attributes(response: Option<&OtelModelValue>) -> Vec<(String, Value)> {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span: Span = provider.tracer("b3856-response").start("case");
    set_response_attributes(&mut span, response);
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

fn attribute<'attributes>(
    attributes: &'attributes [(String, Value)],
    key: &str,
) -> Option<&'attributes Value> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn keys(attributes: &[(String, Value)]) -> Vec<&str> {
    attributes.iter().map(|(key, _)| key.as_str()).collect()
}

#[test]
fn choices_write_the_output_messages_and_finish_reasons() {
    let response = value(
        r#"{"choices": [
            {"message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"},
            {"message": {"role": "assistant", "content": "again"}, "finish_reason": "length"}
        ]}"#,
    );
    let attributes = response_attributes(Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.output.messages"),
        Some(&Value::from(
            r#"[{"finish_reason":"stop","parts":[{"content":"hi","type":"text"}],"role":"assistant"},{"finish_reason":"length","parts":[{"content":"again","type":"text"}],"role":"assistant"}]"#
        ))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.response.finish_reasons"),
        Some(&Value::from(r#"["stop","length"]"#))
    );
}

#[rstest]
#[case::null_reason(
    r#"{"message": {"role": "assistant", "content": "hi"}, "finish_reason": null}"#
)]
#[case::empty_reason(r#"{"message": {"role": "assistant", "content": "hi"}, "finish_reason": ""}"#)]
#[case::no_reason(r#"{"message": {"role": "assistant", "content": "hi"}}"#)]
fn falsy_finish_reasons_are_not_listed(#[case] choice: &str) {
    let response = value(&format!(r#"{{"choices": [{choice}]}}"#));
    let attributes = response_attributes(Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.response.finish_reasons"));
    assert!(keys(&attributes).contains(&"gen_ai.output.messages"));
}

#[test]
fn tool_calls_are_written_for_a_choice_with_a_finish_reason() {
    let response = value(
        r#"{"choices": [{"finish_reason": "tool_calls", "message": {"role": "assistant",
            "tool_calls": [{"id": "call_1", "function": {"name": "lookup", "arguments": "{\"q\":1}"}}]}}]}"#,
    );
    let attributes = response_attributes(Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.completion.0.function_call.name"),
        Some(&Value::from("lookup"))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.completion.0.function_call.arguments"),
        Some(&Value::from(r#"{"q":1}"#))
    );
}

#[test]
fn the_last_tool_calling_choice_wins_each_tool_call_key() {
    let response = value(
        r#"{"choices": [
            {"finish_reason": "tool_calls", "message": {"role": "assistant",
                "tool_calls": [{"id": "call_1", "function": {"name": "first", "arguments": "{}"}}]}},
            {"finish_reason": "tool_calls", "message": {"role": "assistant",
                "tool_calls": [{"id": "call_2", "function": {"name": "second", "arguments": "{}"}}]}}
        ]}"#,
    );
    let attributes = response_attributes(Some(&response));
    let names: Vec<_> = attributes
        .iter()
        .filter(|(key, _)| key == "gen_ai.completion.0.function_call.name")
        .collect();
    assert_eq!(names.len(), 1);
    assert_eq!(
        names.first().map(|(_, found)| found),
        Some(&Value::from("second"))
    );
}

#[test]
fn tool_calls_are_skipped_for_a_choice_without_a_finish_reason() {
    let response = value(
        r#"{"choices": [{"message": {"role": "assistant",
            "tool_calls": [{"id": "call_1", "function": {"name": "lookup"}}]}}]}"#,
    );
    let attributes = response_attributes(Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.completion.0.function_call.name"));
}

#[test]
fn an_empty_choices_list_falls_to_the_responses_output_branch() {
    let response = value(
        r#"{"choices": [],
            "output": [{"type": "message", "role": "assistant",
                        "content": [{"type": "output_text", "text": "hi"}]}],
            "status": "completed"}"#,
    );
    let attributes = response_attributes(Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.output.messages"),
        Some(&Value::from(
            r#"[{"parts":[{"content":"hi","type":"text"}],"role":"assistant"}]"#
        ))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.response.finish_reasons"),
        Some(&Value::from(r#"["completed"]"#))
    );
}

#[test]
fn responses_function_call_items_write_tool_call_attributes() {
    let response = value(
        r#"{"output": [
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hi"}]},
            {"type": "function_call", "name": "lookup", "arguments": "{\"q\":1}", "call_id": "call_1"}
        ]}"#,
    );
    let attributes = response_attributes(Some(&response));
    assert_eq!(
        attribute(&attributes, "gen_ai.completion.0.function_call.name"),
        Some(&Value::from("lookup"))
    );
    assert_eq!(
        attribute(&attributes, "gen_ai.completion.0.function_call.arguments"),
        Some(&Value::from(r#"{"q":1}"#))
    );
}

#[test]
fn a_function_call_without_name_or_arguments_writes_no_tool_call_attributes() {
    let response = value(r#"{"output": [{"type": "function_call"}]}"#);
    let attributes = response_attributes(Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.completion.0.function_call.name"));
    assert!(!keys(&attributes).contains(&"gen_ai.completion.0.function_call.arguments"));
}

#[test]
fn a_responses_output_of_unknown_items_writes_no_output_messages() {
    let response = value(r#"{"output": [{"type": "reasoning"}], "status": "incomplete"}"#);
    let attributes = response_attributes(Some(&response));
    assert!(!keys(&attributes).contains(&"gen_ai.output.messages"));
    assert_eq!(
        attribute(&attributes, "gen_ai.response.finish_reasons"),
        Some(&Value::from(r#"["incomplete"]"#))
    );
}

#[rstest]
#[case::empty_object("{}")]
#[case::no_choices_or_output(r#"{"id": "chatcmpl-1"}"#)]
#[case::null_choices(r#"{"choices": null, "output": null}"#)]
fn a_response_without_choices_or_output_writes_nothing(#[case] raw: &str) {
    let response = value(raw);
    assert!(response_attributes(Some(&response)).is_empty());
}

#[test]
fn an_absent_response_writes_nothing() {
    assert!(response_attributes(None).is_empty());
}
