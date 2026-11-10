#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::langtrace::LangtraceMapper;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "call_type": "completion",
    "stream": true,
    "prompt_tokens": 12,
    "completion_tokens": 5,
    "total_tokens": 17,
    "model_parameters": {
        "temperature": 0.2, "top_p": 0.9, "top_k": 5, "max_tokens": 32,
        "frequency_penalty": 0.1, "presence_penalty": 0.2
    },
    "messages": [
        {"role": "user", "content": "hello"},
        {"role": "tool", "content": "sunny", "tool_call_id": "call_1"}
    ],
    "response": {
        "id": "resp-1",
        "model": "gpt-4o-2024",
        "system_fingerprint": "fp-1",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]
    }
}"#;

fn payload() -> Payload {
    serde_json::from_str(PAYLOAD).expect("payload")
}

fn call() -> LLMCallSpanData {
    LLMCallSpanData::from_standard_logging_payload(
        &payload(),
        LLMCallOptions {
            capture_content: true,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: TraceControls::default(),
            session_id: None,
        },
    )
    .expect("llm call")
}

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
    LangtraceMapper.map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn attributes() -> AttributeMap {
    LangtraceMapper.map(&SpanData::LLMCall(&call()), &BTreeMap::new())
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
fn langtrace_llm_call_writes_the_operation_and_models() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert_eq!(
        attributes.get("langtrace.service.name"),
        Some(&Value::from("openai"))
    );
    assert_eq!(attributes.get("llm.model"), Some(&Value::from("gpt-4o")));
    assert_eq!(
        attributes.get("gen_ai.response.model"),
        Some(&Value::from("gpt-4o-2024"))
    );
    assert_eq!(
        attributes.get("gen_ai.response_id"),
        Some(&Value::from("resp-1"))
    );
    assert_eq!(
        attributes.get("gen_ai.system_fingerprint"),
        Some(&Value::from("fp-1"))
    );
}

#[test]
fn langtrace_llm_call_writes_the_scalar_request_parameters() {
    let attributes = attributes();
    assert_eq!(attributes.get("llm.temperature"), Some(&Value::F64(0.2)));
    assert_eq!(attributes.get("llm.top_p"), Some(&Value::F64(0.9)));
    assert_eq!(attributes.get("llm.top_k"), Some(&Value::I64(5)));
    assert_eq!(attributes.get("llm.max_tokens"), Some(&Value::I64(32)));
    assert_eq!(
        attributes.get("llm.frequency_penalty"),
        Some(&Value::F64(0.1))
    );
    assert_eq!(
        attributes.get("llm.presence_penalty"),
        Some(&Value::F64(0.2))
    );
    assert_eq!(attributes.get("llm.stream"), Some(&Value::Bool(true)));
}

#[test]
fn langtrace_llm_call_writes_the_token_counts() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("llm.token.counts.prompt"),
        Some(&Value::I64(12))
    );
    assert_eq!(
        attributes.get("llm.token.counts.completion"),
        Some(&Value::I64(5))
    );
    assert_eq!(
        attributes.get("llm.token.counts.total"),
        Some(&Value::I64(17))
    );
}

#[test]
fn langtrace_llm_call_writes_the_message_blobs() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "llm.prompts",
        r#"[{"role": "user", "content": "hello"},
            {"role": "tool", "content": "sunny", "tool_call_id": "call_1"}]"#,
    );
    assert_json_attribute(
        &attributes,
        "llm.completions",
        r#"[{"role": "assistant", "content": "done"}]"#,
    );
}

#[test]
fn langtrace_llm_call_keeps_a_null_completion_entry() {
    let attributes = attributes_of(
        r#"{"id": "call-1", "model": "gpt-4o", "call_type": "completion",
            "response": {"choices": [{"finish_reason": "stop"}]}}"#,
        true,
    );
    assert_json_attribute(&attributes, "llm.completions", "[null]");
}

#[test]
fn langtrace_llm_call_omits_absent_values() {
    let attributes = attributes_of(
        r#"{"id": "call-1", "model": "gpt-4o", "call_type": "completion"}"#,
        false,
    );
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert!(!attributes.contains_key("langtrace.service.name"));
    assert!(!attributes.contains_key("llm.temperature"));
    assert!(!attributes.contains_key("llm.stream"));
    assert!(!attributes.contains_key("llm.token.counts.prompt"));
    assert!(!attributes.contains_key("llm.prompts"));
    assert!(!attributes.contains_key("llm.completions"));
    assert!(!attributes.contains_key("gen_ai.response.model"));
}

#[test]
fn langtrace_mapper_answers_only_llm_call_spans() {
    let entry: MetadataMap =
        serde_json::from_str(r#"{"guardrail_name": "pii-guard", "guardrail_status": "success"}"#)
            .expect("entry");
    let guardrail = GuardrailSpanData::from_logging_entry(&entry);
    let attributes = LangtraceMapper.map(&SpanData::Guardrail(&guardrail), &BTreeMap::new());
    assert!(attributes.is_empty());
}
