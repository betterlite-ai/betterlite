#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::weave::WeaveMapper;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData, ServiceType};
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "call_type": "completion",
    "messages": [{"role": "user", "content": "hello"}],
    "response": {
        "id": "resp-1",
        "model": "gpt-4o-2024",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]
    }
}"#;

fn attributes_of(payload_text: &str, capture_content: bool) -> AttributeMap {
    let payload: Payload = serde_json::from_str(payload_text).expect("payload");
    let call = LLMCallSpanData::from_standard_logging_payload(
        &payload,
        LLMCallOptions {
            capture_content,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: TraceControls::default(),
            session_id: None,
        },
    )
    .expect("llm call");
    WeaveMapper.map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn attributes() -> AttributeMap {
    attributes_of(PAYLOAD, true)
}

fn json_attribute(attributes: &AttributeMap, key: &str) -> OtelModelValue {
    let value = attributes.get(key).expect("attribute");
    let Value::String(text) = value else {
        panic!("attribute {key} is not text");
    };
    serde_json::from_str(text.as_str()).expect("json attribute")
}

fn assert_json_attribute(attributes: &AttributeMap, key: &str, expected_text: &str) {
    let actual = json_attribute(attributes, key);
    let expected: OtelModelValue = serde_json::from_str(expected_text).expect("expected");
    assert_eq!(
        serde_json::to_string(&actual).expect("json"),
        serde_json::to_string(&expected).expect("json")
    );
}

#[test]
fn weave_llm_call_writes_the_display_name_and_call_id() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("weave.display_name"),
        Some(&Value::from("chat gpt-4o"))
    );
    assert_eq!(
        attributes.get("weave.call_id"),
        Some(&Value::from("call-1"))
    );
}

#[test]
fn weave_llm_call_writes_the_output_choices() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "weave.output",
        r#"[{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]"#,
    );
}

#[test]
fn weave_llm_call_omits_absent_values() {
    let attributes = attributes_of(r#"{"call_type": "completion"}"#, false);
    assert!(attributes.is_empty());
}

#[test]
fn weave_display_name_uses_the_operation_of_the_call() {
    let attributes = attributes_of(
        r#"{"id": "call-1", "model": "text-embedding-3-small", "call_type": "embedding"}"#,
        false,
    );
    assert_eq!(
        attributes.get("weave.display_name"),
        Some(&Value::from("embeddings text-embedding-3-small"))
    );
    assert!(!attributes.contains_key("weave.output"));
}

#[test]
fn weave_mapper_answers_only_llm_call_spans() {
    let service = ServiceSpanData::from_payload(
        &ServiceLoggerPayload {
            service: ServiceType::Redis,
            call_type: "redis_get".to_owned(),
            caller: None,
            error: None,
        },
        &BTreeMap::new(),
    );
    let attributes = WeaveMapper.map(&SpanData::Service(&service), &BTreeMap::new());
    assert!(attributes.is_empty());
}
