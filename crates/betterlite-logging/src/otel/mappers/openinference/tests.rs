#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::openinference::OpenInferenceMapper;
use crate::otel::mappers::utils::MAX_TOOL_ATTRS_PER_SPAN;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::MCPToolCallSpanData;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "call_type": "completion",
    "prompt_tokens": 10,
    "completion_tokens": 4,
    "total_tokens": 14,
    "model_parameters": {
        "temperature": 0.2, "top_p": 0.9, "top_k": 5, "max_tokens": 32,
        "frequency_penalty": 0.1, "presence_penalty": 0.2, "seed": 7,
        "tools": [
            {"type": "function", "function": {"name": "get_weather", "description": "Weather", "parameters": {"type": "object"}}},
            {"type": "function", "function": {"name": "no_description", "parameters": {"type": "object"}}},
            {"type": "function", "function": {"name": "third_tool", "parameters": {"type": "object"}}}
        ]
    },
    "messages": [
        {"role": "user", "content": [
            {"type": "text", "text": "hello "},
            {"type": "image_url", "image_url": {"url": "https://example.com/x.png"}},
            {"type": "text", "text": "world"}
        ]},
        {"content": "no role"}
    ],
    "response": {
        "id": "resp-1",
        "model": "gpt-4o-2024",
        "choices": [
            {"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"},
            {"finish_reason": "length"}
        ]
    }
}"#;

fn call_from(payload_text: &str, capture_content: bool) -> LLMCallSpanData {
    let payload: Payload = serde_json::from_str(payload_text).expect("payload");
    LLMCallSpanData::from_standard_logging_payload(
        &payload,
        LLMCallOptions {
            capture_content,
            time_to_first_chunk_seconds: None,
            request_route: None,
            trace: TraceControls::default(),
            session_id: None,
        },
    )
    .expect("llm call")
}

fn attributes_with_budget(payload_text: &str, tool_attr_budget: usize) -> AttributeMap {
    let call = call_from(payload_text, true);
    let mapper = OpenInferenceMapper { tool_attr_budget };
    mapper.map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn attributes() -> AttributeMap {
    attributes_with_budget(PAYLOAD, MAX_TOOL_ATTRS_PER_SPAN)
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
fn openinference_llm_call_writes_the_span_kind_and_models() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("openinference.span.kind"),
        Some(&Value::from("LLM"))
    );
    assert_eq!(
        attributes.get("llm.model_name"),
        Some(&Value::from("gpt-4o"))
    );
    assert_eq!(attributes.get("llm.provider"), Some(&Value::from("openai")));
    assert_eq!(
        attributes.get("llm.token_count.prompt"),
        Some(&Value::I64(10))
    );
    assert_eq!(
        attributes.get("llm.token_count.completion"),
        Some(&Value::I64(4))
    );
    assert_eq!(
        attributes.get("llm.token_count.total"),
        Some(&Value::I64(14))
    );
}

#[test]
fn openinference_llm_call_indexes_the_input_messages() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("llm.input_messages.0.message.role"),
        Some(&Value::from("user"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.0.message.content"),
        Some(&Value::from("hello world"))
    );
    assert_eq!(
        attributes.get("llm.input_messages.1.message.content"),
        Some(&Value::from("no role"))
    );
    assert!(!attributes.contains_key("llm.input_messages.1.message.role"));
    assert_json_attribute(
        &attributes,
        "input.value",
        r#"[{"role": "user", "content": "hello world"},
            {"role": null, "content": "no role"}]"#,
    );
}

#[test]
fn openinference_llm_call_indexes_the_output_messages() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("llm.output_messages.0.message.role"),
        Some(&Value::from("assistant"))
    );
    assert_eq!(
        attributes.get("llm.output_messages.0.message.content"),
        Some(&Value::from("done"))
    );
    assert!(!attributes.contains_key("llm.output_messages.1.message.role"));
    assert_json_attribute(
        &attributes,
        "output.value",
        r#"[{"role": "assistant", "content": "done"}, {"role": null, "content": null}]"#,
    );
}

#[test]
fn openinference_llm_call_writes_the_invocation_parameters() {
    let attributes = attributes();
    assert_json_attribute(
        &attributes,
        "llm.invocation_parameters",
        r#"{"temperature": 0.2, "top_p": 0.9, "top_k": 5, "max_tokens": 32,
            "frequency_penalty": 0.1, "presence_penalty": 0.2, "seed": 7}"#,
    );
}

#[test]
fn openinference_llm_call_writes_the_tool_definitions_per_index() {
    let attributes = attributes();
    assert_eq!(
        attributes.get("llm.tools.0.tool.name"),
        Some(&Value::from("get_weather"))
    );
    assert_eq!(
        attributes.get("llm.tools.0.tool.description"),
        Some(&Value::from("Weather"))
    );
    assert_json_attribute(
        &attributes,
        "llm.tools.0.tool.json_schema",
        r#"{"type": "object"}"#,
    );
    assert!(!attributes.contains_key("llm.tools.1.tool.description"));
    assert_eq!(
        attributes.get("llm.tools.2.tool.name"),
        Some(&Value::from("third_tool"))
    );
}

#[test]
fn openinference_llm_call_shares_the_tool_budget_across_fields() {
    let attributes = attributes_with_budget(PAYLOAD, 6);
    assert!(attributes.contains_key("llm.tools.0.tool.name"));
    assert!(attributes.contains_key("llm.tools.1.tool.name"));
    assert!(!attributes.contains_key("llm.tools.2.tool.name"));
}

#[test]
fn openinference_llm_call_omits_absent_values() {
    let call = call_from(
        r#"{"id": "call-1", "model": "gpt-4o", "call_type": "completion"}"#,
        false,
    );
    let mapper = OpenInferenceMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let attributes = mapper.map(&SpanData::LLMCall(&call), &BTreeMap::new());
    assert_eq!(
        attributes.get("openinference.span.kind"),
        Some(&Value::from("LLM"))
    );
    assert!(!attributes.contains_key("llm.provider"));
    assert!(!attributes.contains_key("llm.token_count.prompt"));
    assert!(!attributes.contains_key("llm.invocation_parameters"));
    assert!(!attributes.contains_key("llm.input_messages.0.message.role"));
    assert!(!attributes.contains_key("input.value"));
    assert!(!attributes.contains_key("output.value"));
}

#[test]
fn openinference_mapper_answers_only_llm_call_spans() {
    let payload: Payload =
        serde_json::from_str(r#"{"id": "call-1", "call_type": "call_mcp_tool"}"#).expect("payload");
    let tool_call =
        MCPToolCallSpanData::from_standard_logging_payload(&payload, true).expect("tool call");
    let mapper = OpenInferenceMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let attributes = mapper.map(&SpanData::MCPToolCall(&tool_call), &BTreeMap::new());
    assert!(attributes.is_empty());
}
