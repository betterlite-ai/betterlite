#![cfg(test)]

use opentelemetry::{Array, StringValue, Value};
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::langfuse::LangfuseMapper;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::MCPListToolsSpanData;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "model_group": "chat-group",
    "custom_llm_provider": "openai",
    "call_type": "completion",
    "prompt_tokens": 20,
    "completion_tokens": 10,
    "total_tokens": 30,
    "response_cost": 0.0015,
    "model_parameters": {
        "temperature": 0.2, "top_p": 0.9, "max_tokens": 32,
        "frequency_penalty": 0.1, "presence_penalty": 0.2, "seed": 7
    },
    "messages": [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "hi"}
    ],
    "response": {
        "id": "resp-1",
        "model": "gpt-4o-2024",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]
    },
    "metadata": {
        "user_api_key_team_id": "team-1",
        "user_api_key_team_alias": "Team A",
        "usage_object": {
            "cache_creation_input_tokens": 3,
            "prompt_tokens_details": {"cached_tokens": 5},
            "completion_tokens_details": {"reasoning_tokens": 2}
        }
    }
}"#;

fn payload_text(text: &str) -> Payload {
    serde_json::from_str(text).expect("payload")
}

fn trace_controls() -> TraceControls {
    TraceControls {
        name: Some("trace-1".to_owned()),
        user_id: Some("user-1".to_owned()),
        session_id: Some("session-1".to_owned()),
        tags: vec!["env:test".to_owned(), "beta".to_owned()],
    }
}

fn call_with(payload_text_value: &str, capture_content: bool) -> LLMCallSpanData {
    LLMCallSpanData::from_standard_logging_payload(
        &payload_text(payload_text_value),
        LLMCallOptions {
            capture_content,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: trace_controls(),
            session_id: None,
        },
    )
    .expect("llm call")
}

fn attributes_of(payload_text_value: &str) -> AttributeMap {
    let call = call_with(payload_text_value, true);
    LangfuseMapper.map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn attributes_of_default_trace(payload_text_value: &str) -> AttributeMap {
    let call = LLMCallSpanData::from_standard_logging_payload(
        &payload_text(payload_text_value),
        LLMCallOptions {
            capture_content: true,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: TraceControls::default(),
            session_id: None,
        },
    )
    .expect("llm call");
    LangfuseMapper.map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn attributes() -> AttributeMap {
    attributes_of(PAYLOAD)
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
fn langfuse_llm_call_writes_the_observation_identity() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("langfuse.observation.type"),
        Some(&Value::from("generation"))
    );
    assert_eq!(
        attributes.get("langfuse.observation.model.name"),
        Some(&Value::from("chat-group"))
    );
    assert_eq!(
        attributes.get("langfuse.observation.metadata.provider"),
        Some(&Value::from("openai"))
    );
    assert_eq!(
        attributes.get("langfuse.observation.id"),
        Some(&Value::from("call-1"))
    );
    assert_eq!(
        attributes.get("langfuse.trace.metadata.team_id"),
        Some(&Value::from("team-1"))
    );
    assert_eq!(
        attributes.get("langfuse.trace.metadata.team_alias"),
        Some(&Value::from("Team A"))
    );
}

#[test]
fn langfuse_llm_call_writes_the_trace_controls() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("langfuse.trace.name"),
        Some(&Value::from("trace-1"))
    );
    assert_eq!(attributes.get("user.id"), Some(&Value::from("user-1")));
    assert_eq!(
        attributes.get("session.id"),
        Some(&Value::from("session-1"))
    );
    assert_eq!(
        attributes.get("langfuse.trace.tags"),
        Some(&Value::Array(Array::String(vec![
            StringValue::from("env:test".to_owned()),
            StringValue::from("beta".to_owned()),
        ])))
    );
}

#[test]
fn langfuse_llm_call_writes_the_usage_breakdown() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "langfuse.observation.usage_details",
        r#"{"input": 12, "input_cached_tokens": 5, "input_cache_creation": 3,
            "output": 8, "output_reasoning_tokens": 2, "total": 30}"#,
    );
}

#[test]
fn langfuse_llm_call_writes_the_cost_and_model_parameters() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "langfuse.observation.cost_details",
        r#"{"total": 0.0015}"#,
    );
    assert_json_attribute(
        &attributes,
        "langfuse.observation.model.parameters",
        r#"{"temperature": 0.2, "top_p": 0.9, "max_tokens": 32,
            "frequency_penalty": 0.1, "presence_penalty": 0.2, "seed": 7}"#,
    );
}

#[test]
fn langfuse_llm_call_writes_the_message_blobs() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "langfuse.observation.input",
        r#"[{"role": "user", "content": "hello"}, {"role": "assistant", "content": "hi"}]"#,
    );
    assert_json_attribute(
        &attributes,
        "langfuse.observation.output",
        r#"[{"role": "assistant", "content": "done"}]"#,
    );
}

#[test]
fn langfuse_llm_call_writes_only_the_embedding_summary_for_embeddings() {
    let attributes = attributes_of(
        r#"{"id": "call-1", "model": "text-embedding-3-small", "custom_llm_provider": "openai",
            "call_type": "embedding", "response": {"data": [
                {"embedding": [0.1, 0.2, 0.3]}, {"embedding": [0.4, 0.5, 0.6]}]}}"#,
    );
    assert_json_attribute(
        &attributes,
        "langfuse.observation.output",
        r#"{"count": 2, "dimensions": 3}"#,
    );
}

#[test]
fn langfuse_llm_call_omits_absent_values() {
    let attributes = attributes_of_default_trace(r#"{"id": "call-1"}"#);
    assert_eq!(
        attributes.get("langfuse.observation.type"),
        Some(&Value::from("generation"))
    );
    assert!(!attributes.contains_key("langfuse.observation.model.name"));
    assert!(!attributes.contains_key("langfuse.observation.metadata.provider"));
    assert!(!attributes.contains_key("langfuse.trace.metadata.team_id"));
    assert!(!attributes.contains_key("langfuse.observation.usage_details"));
    assert!(!attributes.contains_key("langfuse.observation.cost_details"));
    assert!(!attributes.contains_key("langfuse.observation.model.parameters"));
    assert!(!attributes.contains_key("langfuse.observation.input"));
    assert!(!attributes.contains_key("langfuse.observation.output"));
    assert!(!attributes.contains_key("langfuse.trace.tags"));
}

#[test]
fn langfuse_mapper_answers_only_llm_call_spans() {
    let payload: Payload =
        serde_json::from_str(r#"{"id": "call-1", "call_type": "call_mcp_tool"}"#).expect("payload");
    let list_tools = MCPListToolsSpanData::from_standard_logging_payload(&payload).expect("span");
    let attributes = LangfuseMapper.map(&SpanData::MCPListTools(&list_tools), &BTreeMap::new());
    assert!(attributes.is_empty());
}

#[test]
fn langfuse_usage_details_keep_an_explicit_zero() {
    let attributes = attributes_of(
        r#"{"id": "call-1", "model": "gpt-4o", "prompt_tokens": 4, "completion_tokens": 2,
            "metadata": {"usage_object": {"prompt_tokens_details": {"cached_tokens": 0}}}}"#,
    );
    assert_json_attribute(
        &attributes,
        "langfuse.observation.usage_details",
        r#"{"input": 4, "input_cached_tokens": 0, "output": 2}"#,
    );
}
