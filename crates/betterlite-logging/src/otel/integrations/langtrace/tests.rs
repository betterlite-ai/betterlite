#![cfg(test)]

use opentelemetry::Value;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use std::collections::BTreeMap;

use crate::otel::integrations::langtrace::{LangtraceAttributes, langtrace_trace_endpoint};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

struct Recorded {
    attributes: BTreeMap<String, Value>,
    events: Vec<(String, BTreeMap<String, Value>)>,
}

#[rstest::rstest]
#[case::default_host(None, "https://app.langtrace.ai/api/trace")]
#[case::trailing_slash(
    Some("https://langtrace.example/"),
    "https://langtrace.example/api/trace"
)]
#[case::trace_path_kept(
    Some("https://langtrace.example/api/trace"),
    "https://langtrace.example/api/trace"
)]
#[case::trace_path_after_slashes(
    Some("https://langtrace.example/api/trace///"),
    "https://langtrace.example/api/trace"
)]
fn langtrace_endpoint_appends_the_trace_path(
    #[case] api_host: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(langtrace_trace_endpoint(api_host), expected);
}

fn map(text: &str) -> MetadataMap {
    serde_json::from_str(text).expect("map")
}

fn recorded(kwargs: &MetadataMap, response: &MetadataMap) -> Recorded {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let mut span = provider.tracer("langtrace").start("test");
    LangtraceAttributes::set_langtrace_attributes(&mut span, kwargs, response);
    span.end();
    let spans = exporter.get_finished_spans().expect("spans");
    let finished = spans.first().expect("one span");
    Recorded {
        attributes: finished
            .attributes
            .iter()
            .map(|pair| (pair.key.to_string(), pair.value.clone()))
            .collect(),
        events: finished
            .events
            .events
            .iter()
            .map(|event| {
                (
                    event.name.to_string(),
                    event
                        .attributes
                        .iter()
                        .map(|pair| (pair.key.to_string(), pair.value.clone()))
                        .collect(),
                )
            })
            .collect(),
    }
}

fn json_text(value: &Value) -> OtelModelValue {
    let Value::String(text) = value else {
        panic!("event attribute is not text");
    };
    serde_json::from_str(text.as_str()).expect("json attribute")
}

fn assert_json_text(value: &Value, expected_text: &str) {
    let actual = json_text(value);
    let expected: OtelModelValue = serde_json::from_str(expected_text).expect("expected");
    assert_eq!(
        serde_json::to_string(&actual).expect("json"),
        serde_json::to_string(&expected).expect("json")
    );
}

const REQUEST_KWARGS: &str = r#"{
    "model": "gpt-4o",
    "stream": true,
    "temperature": 0.2,
    "top_p": 0.9,
    "top_k": 5,
    "user": "user-1",
    "max_tokens": 64,
    "stop": ["END", "STOP"],
    "frequency_penalty": 0.1,
    "presence_penalty": 0.2,
    "messages": [{"role": "user", "content": "hi"}],
    "litellm_params": {"custom_llm_provider": "openai"},
    "optional_params": {"temperature": 0.3, "top_k": 7}
}"#;

const RESPONSE: &str = r#"{
    "id": "resp-1",
    "model": "gpt-4o-2024",
    "system_fingerprint": "fp-1",
    "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}],
    "usage": {"prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17}
}"#;

#[test]
fn langtrace_request_attributes_write_the_operation_and_scalars() {
    let recorded = recorded(&map(REQUEST_KWARGS), &map("{}"));
    let attributes = &recorded.attributes;
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert_eq!(
        attributes.get("langtrace.service.name"),
        Some(&Value::from("openai"))
    );
    assert_eq!(
        attributes.get("gen_ai.request.model"),
        Some(&Value::from("gpt-4o"))
    );
    assert_eq!(attributes.get("llm.is_streaming"), Some(&Value::Bool(true)));
    assert_eq!(attributes.get("llm.user"), Some(&Value::from("user-1")));
    assert_eq!(
        attributes.get("gen_ai.request.max_tokens"),
        Some(&Value::I64(64))
    );
}

#[test]
fn langtrace_request_attributes_merge_the_optional_params_last() {
    let recorded = recorded(&map(REQUEST_KWARGS), &map("{}"));
    let attributes = &recorded.attributes;
    assert_eq!(
        attributes.get("gen_ai.request.temperature"),
        Some(&Value::F64(0.3))
    );
    assert_eq!(attributes.get("llm.top_k"), Some(&Value::I64(7)));
    assert_eq!(
        attributes.get("gen_ai.request.top_p"),
        Some(&Value::F64(0.9))
    );
    assert_eq!(
        attributes.get("llm.response.stop_reason"),
        Some(&Value::Array(opentelemetry::Array::String(vec![
            opentelemetry::StringValue::from("END".to_owned()),
            opentelemetry::StringValue::from("STOP".to_owned()),
        ])))
    );
    assert_eq!(
        attributes.get("llm.frequency_penalty"),
        Some(&Value::F64(0.1))
    );
    assert_eq!(
        attributes.get("llm.presence_penalty"),
        Some(&Value::F64(0.2))
    );
}

#[test]
fn langtrace_request_event_carries_the_prompt() {
    let recorded = recorded(&map(REQUEST_KWARGS), &map("{}"));
    let (name, attributes) = recorded.events.first().expect("prompt event");
    assert_eq!(name, "gen_ai.content.prompt");
    assert_json_text(
        attributes.get("gen_ai.prompt").expect("prompt attribute"),
        r#"[{"role": "user", "content": "hi"}]"#,
    );
}

#[test]
fn langtrace_response_attributes_and_event_are_written() {
    let recorded = recorded(&map("{}"), &map(RESPONSE));
    assert_eq!(
        recorded.attributes.get("gen_ai.response_id"),
        Some(&Value::from("resp-1"))
    );
    assert_eq!(
        recorded.attributes.get("gen_ai.system_fingerprint"),
        Some(&Value::from("fp-1"))
    );
    assert_eq!(
        recorded.attributes.get("gen_ai.response.model"),
        Some(&Value::from("gpt-4o-2024"))
    );
    let (name, attributes) = recorded.events.first().expect("completion event");
    assert_eq!(name, "gen_ai.content.completion");
    assert_json_text(
        attributes
            .get("gen_ai.completion")
            .expect("completion attribute"),
        r#"[{"role": "assistant", "content": "done"}]"#,
    );
}

#[test]
fn langtrace_usage_attributes_write_the_token_counts() {
    let recorded = recorded(&map("{}"), &map(RESPONSE));
    assert_eq!(
        recorded.attributes.get("gen_ai.usage.prompt_tokens"),
        Some(&Value::I64(12))
    );
    assert_eq!(
        recorded.attributes.get("gen_ai.usage.completion_tokens"),
        Some(&Value::I64(5))
    );
    assert_eq!(
        recorded.attributes.get("llm.usage.total_tokens"),
        Some(&Value::I64(17))
    );
}

#[test]
fn langtrace_writes_an_empty_completion_event_without_a_response() {
    let recorded = recorded(&map("{}"), &map("{}"));
    assert_eq!(
        recorded.attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert!(!recorded.attributes.contains_key("langtrace.service.name"));
    let (name, attributes) = recorded.events.first().expect("completion event");
    assert_eq!(name, "gen_ai.content.completion");
    assert_json_text(
        attributes
            .get("gen_ai.completion")
            .expect("completion attribute"),
        "[]",
    );
}

#[test]
fn langtrace_keeps_an_explicit_false_stream() {
    let kwargs = map(
        r#"{"model": "gpt-4o", "stream": false, "litellm_params": {"custom_llm_provider": "openai"}}"#,
    );
    let recorded = recorded(&kwargs, &map("{}"));
    assert_eq!(
        recorded.attributes.get("llm.is_streaming"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn langtrace_skips_a_choice_without_a_message() {
    let response = map(
        r#"{"choices": [{"finish_reason": "stop"}, {"message": {"role": "assistant", "content": "done"}}]}"#,
    );
    let recorded = recorded(&map("{}"), &response);
    let (name, attributes) = recorded.events.first().expect("completion event");
    assert_eq!(name, "gen_ai.content.completion");
    assert_json_text(
        attributes
            .get("gen_ai.completion")
            .expect("completion attribute"),
        r#"[{"role": "assistant", "content": "done"}]"#,
    );
}

#[test]
fn langtrace_usage_attributes_are_skipped_without_usage() {
    let recorded = recorded(&map("{}"), &map(r#"{"id": "resp-1"}"#));
    assert!(
        !recorded
            .attributes
            .contains_key("gen_ai.usage.prompt_tokens")
    );
    assert!(!recorded.attributes.contains_key("llm.usage.total_tokens"));
}
