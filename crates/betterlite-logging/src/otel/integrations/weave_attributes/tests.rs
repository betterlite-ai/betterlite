#![cfg(test)]

use std::collections::BTreeMap;

use super::set_weave_otel_attributes;
use crate::otel::integrations::weave_attributes::keys;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, Span as SdkSpan};

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn value(text: &str) -> OtelModelValue {
    serde_json::from_str(text).expect("value")
}

fn weave_attributes(kwargs: &str, response: &str) -> BTreeMap<String, Value> {
    let source = map(kwargs);
    let response_value = value(response);
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("weave-attributes").start("test");
    set_weave_otel_attributes(&mut span, &source, &response_value, false);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let finished = spans.first().expect("one span");
    finished
        .attributes
        .iter()
        .map(|pair| (pair.key.to_string(), pair.value.clone()))
        .collect::<BTreeMap<_, _>>()
}

fn json_text(attributes: &BTreeMap<String, Value>, key: &str) -> String {
    let Some(Value::String(text)) = attributes.get(key) else {
        panic!("{key} is not a string attribute");
    };
    text.as_str().to_owned()
}

fn expected_json(text: &str) -> String {
    serde_json::to_string(&value(text)).expect("json text")
}

#[test]
fn weave_messages_prompt_includes_functions_and_tools() {
    let attributes = weave_attributes(
        r#"{"messages":[{"role":"user","content":"hi"}],"optional_params":{"functions":[{"name":"legacy"}],"tools":[{"type":"function","function":{"name":"lookup"}}]},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        json_text(&attributes, "input.value"),
        expected_json(
            r#"{"messages":[{"role":"user","content":"hi"}],"functions":[{"name":"legacy"}],"tools":[{"type":"function","function":{"name":"lookup"}}]}"#
        )
    );
}

#[test]
fn weave_messages_default_to_an_empty_array() {
    let attributes = weave_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        json_text(&attributes, "input.value"),
        expected_json(r#"{"messages":[]}"#)
    );
}

#[test]
fn weave_messages_that_are_not_an_array_become_an_empty_array() {
    let attributes = weave_attributes(
        r#"{"messages":"not an array","standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        json_text(&attributes, "input.value"),
        expected_json(r#"{"messages":[]}"#)
    );
}

#[test]
fn weave_null_functions_and_tools_are_omitted() {
    let attributes = weave_attributes(
        r#"{"optional_params":{"functions":null,"tools":null},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        json_text(&attributes, "input.value"),
        expected_json(r#"{"messages":[]}"#)
    );
}

#[test]
fn weave_display_name_from_provider_and_model() {
    let attributes = weave_attributes(
        r#"{"model":"gpt-4o","litellm_params":{"custom_llm_provider":"openai"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::DISPLAY_NAME),
        Some(&Value::from("openai__gpt-4o"))
    );
}

#[test]
fn weave_display_name_without_provider_is_the_model() {
    let attributes = weave_attributes(
        r#"{"model":"gpt-4o","standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::DISPLAY_NAME),
        Some(&Value::from("gpt-4o"))
    );
}

#[test]
fn weave_display_name_from_metadata_wins_and_keeps_its_slashes_replaced() {
    let attributes = weave_attributes(
        r#"{"model":"gpt-4o","metadata":{"display_name":"My Project/Name"},"litellm_params":{"custom_llm_provider":"openai"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::DISPLAY_NAME),
        Some(&Value::from("My Project__Name"))
    );
}

#[test]
fn weave_display_name_replaces_every_slash() {
    let attributes = weave_attributes(
        r#"{"model":"org/model","litellm_params":{"custom_llm_provider":"openai"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::DISPLAY_NAME),
        Some(&Value::from("openai__org__model"))
    );
}

#[test]
fn weave_empty_display_name_falls_back_to_the_model() {
    let attributes = weave_attributes(
        r#"{"model":"gpt-4o","metadata":{"display_name":""},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::DISPLAY_NAME),
        Some(&Value::from("gpt-4o"))
    );
}

#[test]
fn weave_display_name_is_skipped_without_a_model() {
    let attributes = weave_attributes(
        r#"{"metadata":{"display_name":""},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert!(!attributes.contains_key(keys::DISPLAY_NAME));
}

#[test]
fn weave_session_id_string_sets_thread_id_and_turn() {
    let attributes = weave_attributes(
        r#"{"metadata":{"session_id":"session-1"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::THREAD_ID),
        Some(&Value::from("session-1"))
    );
    assert_eq!(attributes.get(keys::IS_TURN), Some(&Value::Bool(true)));
}

#[test]
fn weave_session_id_object_is_written_as_json_text() {
    let attributes = weave_attributes(
        r#"{"metadata":{"session_id":{"thread":1}},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get(keys::THREAD_ID),
        Some(&Value::from(r#"{"thread":1}"#))
    );
    assert_eq!(attributes.get(keys::IS_TURN), Some(&Value::Bool(true)));
}

#[test]
fn weave_session_id_array_is_written_as_json_text() {
    let attributes = weave_attributes(
        r#"{"metadata":{"session_id":[1,2]},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(attributes.get(keys::THREAD_ID), Some(&Value::from("[1,2]")));
}

#[test]
fn weave_null_session_id_writes_nothing() {
    let attributes = weave_attributes(
        r#"{"metadata":{"session_id":null},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert!(!attributes.contains_key(keys::THREAD_ID));
    assert!(!attributes.contains_key(keys::IS_TURN));
}

#[test]
fn weave_empty_session_id_is_still_written() {
    let attributes = weave_attributes(
        r#"{"metadata":{"session_id":""},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(attributes.get(keys::THREAD_ID), Some(&Value::from("")));
    assert_eq!(attributes.get(keys::IS_TURN), Some(&Value::Bool(true)));
}

#[test]
fn weave_output_replaces_the_shared_response_projection() {
    let response =
        r#"{"id":"chatcmpl-1","choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
    let attributes = weave_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion"}}"#,
        response,
    );
    assert_eq!(
        json_text(&attributes, "output.value"),
        expected_json(response)
    );
}

#[test]
fn weave_empty_response_writes_no_output() {
    let attributes = weave_attributes(
        r#"{"standard_logging_object":{"call_type":"acompletion"}}"#,
        "{}",
    );
    assert!(!attributes.contains_key("output.value"));
    assert!(!attributes.contains_key(keys::DISPLAY_NAME));
    assert!(!attributes.contains_key(keys::THREAD_ID));
}

#[test]
fn weave_flow_still_writes_the_shared_attributes() {
    let attributes = weave_attributes(
        r#"{"litellm_params":{"custom_llm_provider":"openai"},"standard_logging_object":{"call_type":"acompletion"}}"#,
        "null",
    );
    assert_eq!(
        attributes.get("openinference.span.kind"),
        Some(&Value::from("LLM"))
    );
    assert_eq!(attributes.get("llm.provider"), Some(&Value::from("openai")));
    assert_eq!(
        attributes.get("llm.request.type"),
        Some(&Value::from("acompletion"))
    );
}

#[test]
fn weave_missing_standard_logging_object_writes_span_kind_and_exception() {
    let kwargs = map(r#"{"model":"gpt-4o"}"#);
    let response = value("null");
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span: SdkSpan = provider.tracer("weave-attributes").start("test");
    set_weave_otel_attributes(&mut span, &kwargs, &response, false);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let finished = spans.first().expect("one span");
    assert!(
        finished
            .events
            .events
            .iter()
            .any(|event| event.name == "exception")
    );
    assert_eq!(
        finished
            .attributes
            .iter()
            .find(|pair| pair.key.as_str() == "openinference.span.kind")
            .map(|pair| pair.value.clone()),
        Some(Value::from("UNKNOWN"))
    );
}
