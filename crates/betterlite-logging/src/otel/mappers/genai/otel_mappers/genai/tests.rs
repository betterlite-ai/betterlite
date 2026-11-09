#![cfg(test)]

use opentelemetry::{Array, StringValue, Value};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::str::FromStr as _;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper as _, SpanData};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::utils::{MAX_TOOL_ATTRS_PER_SPAN, tool_attr_budget};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::{MCPListToolsSpanData, MCPToolCallSpanData};
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData, ServiceType};
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "model_group": "chat-group",
    "api_base": "https://api.openai.com/v1",
    "prompt_tokens": 10,
    "completion_tokens": 4,
    "total_tokens": 14,
    "response_cost": 0.0015,
    "custom_llm_provider": "openai",
    "call_type": "completion",
    "stream": true,
    "model_parameters": {
        "temperature": 0.2,
        "top_p": 0.9,
        "top_k": 5,
        "max_tokens": 32,
        "frequency_penalty": 0.1,
        "presence_penalty": 0.2,
        "stop": ["END", "STOP"],
        "seed": 7,
        "tools": [
            {"type": "function", "function": {"name": "get_weather", "description": "Weather", "parameters": {"type": "object"}}},
            {"type": "function", "function": {"name": "no_description", "parameters": {"type": "object"}}}
        ]
    },
    "cost_breakdown": {"input_cost": 0.001, "output_cost": 0.0005},
    "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}],
    "response": {
        "id": "resp-1",
        "model": "gpt-4o-2024",
        "choices": [{"message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}]
    }
}"#;

fn payload() -> Payload {
    serde_json::from_str(PAYLOAD).expect("payload")
}

fn genai_mapper() -> GenAIMapper {
    GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    }
}

fn captured_llm_call() -> LLMCallSpanData {
    LLMCallSpanData::from_standard_logging_payload(
        &payload(),
        LLMCallOptions {
            capture_content: true,
            time_to_first_chunk_seconds: Some(Decimal::from_str("1.5").expect("decimal")),
            request_route: Some("/v1/chat/completions".to_owned()),
            trace: TraceControls::default(),
            session_id: Some("session-1".to_owned()),
        },
    )
    .expect("llm call span")
}

fn attributes_of_call() -> AttributeMap {
    let call = captured_llm_call();
    genai_mapper().map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

fn assert_json_attribute(attributes: &AttributeMap, key: &str, expected_text: &str) {
    let value = attributes.get(key).expect("attribute");
    let Value::String(text) = value else {
        panic!("attribute {key} is not text");
    };
    let actual: OtelModelValue = serde_json::from_str(text.as_str()).expect("json attribute");
    let expected: OtelModelValue = serde_json::from_str(expected_text).expect("expected");
    assert_eq!(
        serde_json::to_string(&actual).expect("json"),
        serde_json::to_string(&expected).expect("json")
    );
}

#[test]
fn genai_llm_call_writes_the_request_attributes() {
    let attributes = attributes_of_call();
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert_eq!(
        attributes.get("gen_ai.provider.name"),
        Some(&Value::from("openai"))
    );
    assert_eq!(
        attributes.get("gen_ai.conversation.id"),
        Some(&Value::from("session-1"))
    );
    assert_eq!(
        attributes.get("gen_ai.request.model"),
        Some(&Value::from("chat-group"))
    );
    assert_eq!(
        attributes.get("gen_ai.request.temperature"),
        Some(&Value::F64(0.2))
    );
    assert_eq!(
        attributes.get("gen_ai.request.max_tokens"),
        Some(&Value::I64(32))
    );
    assert_eq!(attributes.get("gen_ai.request.seed"), Some(&Value::I64(7)));
    assert_eq!(
        attributes.get("gen_ai.request.stop_sequences"),
        Some(&Value::Array(Array::String(vec![
            StringValue::from("END".to_owned()),
            StringValue::from("STOP".to_owned()),
        ])))
    );
}

#[test]
fn genai_llm_call_writes_the_response_attributes() {
    let attributes = attributes_of_call();
    assert_eq!(
        attributes.get("gen_ai.response.model"),
        Some(&Value::from("gpt-4o-2024"))
    );
    assert_eq!(
        attributes.get("gen_ai.response.id"),
        Some(&Value::from("resp-1"))
    );
    assert_eq!(
        attributes.get("gen_ai.response.finish_reasons"),
        Some(&Value::Array(Array::String(vec![StringValue::from(
            "stop".to_owned()
        )])))
    );
    assert_eq!(
        attributes.get("gen_ai.response.time_to_first_chunk"),
        Some(&Value::F64(1.5))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.input_tokens"),
        Some(&Value::I64(10))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.output_tokens"),
        Some(&Value::I64(4))
    );
    assert_eq!(
        attributes.get("litellm.request.streaming"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        attributes.get("litellm.request.route"),
        Some(&Value::from("/v1/chat/completions"))
    );
}

#[test]
fn genai_llm_call_writes_identity_and_cost() {
    let attributes = attributes_of_call();
    assert_eq!(
        attributes.get("litellm.call_id"),
        Some(&Value::from("call-1"))
    );
    assert_eq!(
        attributes.get("litellm.call_type"),
        Some(&Value::from("completion"))
    );
    assert_eq!(
        attributes.get("litellm.provider.model"),
        Some(&Value::from("gpt-4o"))
    );
    assert_eq!(
        attributes.get("litellm.cost.total"),
        Some(&Value::F64(0.0015))
    );
    assert_eq!(
        attributes.get("litellm.cost.input"),
        Some(&Value::F64(0.001))
    );
    assert_eq!(
        attributes.get("litellm.cost.output"),
        Some(&Value::F64(0.0005))
    );
    assert!(!attributes.contains_key("litellm.cost.original"));
}

#[test]
fn genai_llm_call_writes_messages_and_tools() {
    let attributes = attributes_of_call();
    assert_json_attribute(
        &attributes,
        "gen_ai.input.messages",
        r#"[{"role": "user", "content": [{"type": "text", "text": "hi"}]}]"#,
    );
    assert_json_attribute(
        &attributes,
        "gen_ai.output.messages",
        r#"[{"role": "assistant", "content": "done"}]"#,
    );
    assert_eq!(
        attributes.get("litellm.request.tools.declared"),
        Some(&Value::I64(2))
    );
    assert_eq!(
        attributes.get("gen_ai.tool.0.name"),
        Some(&Value::from("get_weather"))
    );
    assert_eq!(
        attributes.get("gen_ai.tool.1.description"),
        None,
        "a missing description is dropped"
    );
}

#[test]
fn genai_tool_budget_keeps_only_the_first_tools() {
    let mut call = captured_llm_call();
    call.tools = (0..4_usize)
        .map(|index| ToolDefinition {
            name: format!("tool-{index}"),
            description: None,
            parameters_json: None,
        })
        .collect();
    let mapper = GenAIMapper {
        tool_attr_budget: tool_attr_budget(4),
    };
    let attributes = mapper.map(&SpanData::LLMCall(&call), &BTreeMap::new());
    assert_eq!(
        attributes.get("litellm.request.tools.declared"),
        Some(&Value::I64(4))
    );
    assert!(attributes.contains_key("gen_ai.tool.1.name"));
    assert!(!attributes.contains_key("gen_ai.tool.2.name"));
}

fn postgres_service() -> ServiceSpanData {
    ServiceSpanData::from_payload(
        &ServiceLoggerPayload {
            service: ServiceType::Postgres,
            call_type: "postgres_query".to_owned(),
            caller: Some("litellm.completion".to_owned()),
            error: None,
        },
        &BTreeMap::from([(
            "batch_size".to_owned(),
            OtelModelValue::String("10".to_owned()),
        )]),
    )
}

fn postgres_environment() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "DATABASE_URL".to_owned(),
        "postgresql://user:pass@db.example.com:5433/app?schema=tenant".to_owned(),
    )])
}

#[test]
fn genai_service_span_writes_the_service_attributes() {
    let attributes = genai_mapper().map(
        &SpanData::Service(&postgres_service()),
        &postgres_environment(),
    );
    assert_eq!(
        attributes.get("litellm.service.name"),
        Some(&Value::from("postgres"))
    );
    assert_eq!(
        attributes.get("litellm.service.call_type"),
        Some(&Value::from("postgres_query"))
    );
    assert_eq!(
        attributes.get("litellm.service.caller"),
        Some(&Value::from("litellm.completion"))
    );
    assert_eq!(
        attributes.get("litellm.metadata.batch_size"),
        Some(&Value::from("10"))
    );
}

#[test]
fn genai_service_span_writes_the_datastore_attributes() {
    let attributes = genai_mapper().map(
        &SpanData::Service(&postgres_service()),
        &postgres_environment(),
    );
    assert_eq!(
        attributes.get("db.system.name"),
        Some(&Value::from("postgresql"))
    );
    assert_eq!(
        attributes.get("db.operation.name"),
        Some(&Value::from("postgres_query"))
    );
    assert_eq!(
        attributes.get("server.address"),
        Some(&Value::from("db.example.com"))
    );
    assert_eq!(attributes.get("server.port"), Some(&Value::I64(5433)));
    assert_eq!(
        attributes.get("db.namespace"),
        Some(&Value::from("app|tenant"))
    );
}

fn guardrail_entry() -> MetadataMap {
    serde_json::from_str(
        r#"{
            "guardrail_name": "pii-guard",
            "guardrail_mode": "pre_call",
            "guardrail_status": "guardrail_intervened",
            "masked_entity_count": {"EMAIL": 2},
            "guardrail_provider": "presidio",
            "guardrail_action": "block",
            "guardrail_response": {"verdict": "blocked"},
            "violation_categories": ["pii"],
            "confidence_score": 0.91,
            "guardrail_cost": 0.012,
            "guardrail_cost_in_spend": false
        }"#,
    )
    .expect("entry")
}

fn guardrail_attributes() -> AttributeMap {
    let guardrail = GuardrailSpanData::from_logging_entry(&guardrail_entry());
    genai_mapper().map(&SpanData::Guardrail(&guardrail), &BTreeMap::new())
}

#[test]
fn genai_guardrail_span_writes_the_status_attributes() {
    let attributes = guardrail_attributes();
    assert_eq!(
        attributes.get("litellm.guardrail.name"),
        Some(&Value::from("pii-guard"))
    );
    assert_eq!(
        attributes.get("litellm.guardrail.mode"),
        Some(&Value::from("pre_call"))
    );
    assert_eq!(
        attributes.get("litellm.guardrail.status"),
        Some(&Value::from("guardrail_intervened"))
    );
    assert_eq!(
        attributes.get("litellm.guardrail.masked_entity_count"),
        Some(&Value::I64(2))
    );
    assert_eq!(
        attributes.get("litellm.guardrail.confidence_score"),
        Some(&Value::F64(0.91))
    );
}

#[test]
fn genai_guardrail_span_writes_the_verdict_attributes() {
    let attributes = guardrail_attributes();
    assert_json_attribute(
        &attributes,
        "litellm.guardrail.response",
        r#"{"verdict": "blocked"}"#,
    );
    assert_eq!(
        attributes.get("litellm.guardrail.violation_categories"),
        Some(&Value::Array(Array::String(vec![StringValue::from(
            "pii".to_owned()
        )])))
    );
    assert_eq!(
        attributes.get("litellm.cost.guardrail"),
        Some(&Value::F64(0.012))
    );
    assert_eq!(
        attributes.get("litellm.guardrail.cost_in_spend"),
        Some(&Value::Bool(false))
    );
}

fn mcp_payload() -> Payload {
    serde_json::from_str(
        r#"{
            "id": "call-1",
            "call_type": "call_mcp_tool",
            "response_cost": 0.002,
            "metadata": {"mcp_tool_call_metadata": {
                "name": "get_weather",
                "mcp_server_name": "weather",
                "mcp_server_resource": "https://weather.example:8443",
                "arguments": {"city": "Paris"}
            }}
        }"#,
    )
    .expect("payload")
}

#[test]
fn genai_mcp_spans_write_only_their_own_keys() {
    let tool_call = MCPToolCallSpanData::from_standard_logging_payload(&mcp_payload(), true)
        .expect("tool call");
    let attributes = genai_mapper().map(&SpanData::MCPToolCall(&tool_call), &BTreeMap::new());
    assert_eq!(
        attributes.get("gen_ai.operation.name"),
        Some(&Value::from("execute_tool"))
    );
    assert_eq!(attributes.get("rpc.system"), Some(&Value::from("jsonrpc")));
    assert_eq!(attributes.get("server.port"), Some(&Value::I64(8443)));
    assert_eq!(
        attributes.get("litellm.cost.total"),
        Some(&Value::F64(0.002))
    );
    assert!(!attributes.contains_key("litellm.guardrail.name"));
    assert!(!attributes.contains_key("litellm.service.name"));
}

#[test]
fn genai_mcp_list_tools_omits_the_tool_keys() {
    let list_tools =
        MCPListToolsSpanData::from_standard_logging_payload(&mcp_payload()).expect("list tools");
    let attributes = genai_mapper().map(&SpanData::MCPListTools(&list_tools), &BTreeMap::new());
    assert_eq!(
        attributes.get("mcp.method.name"),
        Some(&Value::from("tools/list"))
    );
    assert!(!attributes.contains_key("gen_ai.operation.name"));
    assert!(!attributes.contains_key("gen_ai.tool.name"));
}

#[test]
fn genai_omits_absent_values() {
    let sparse_payload: Payload =
        serde_json::from_str(r#"{"id": "call-1", "model": "gpt-4o"}"#).expect("payload");
    let call =
        LLMCallSpanData::from_standard_logging_payload(&sparse_payload, LLMCallOptions::default())
            .expect("llm call");
    let attributes = genai_mapper().map(&SpanData::LLMCall(&call), &BTreeMap::new());
    assert!(!attributes.contains_key("gen_ai.provider.name"));
    assert!(!attributes.contains_key("error.type"));
    assert!(!attributes.contains_key("server.address"));
    assert!(!attributes.contains_key("gen_ai.request.stop_sequences"));
    assert!(!attributes.contains_key("gen_ai.response.finish_reasons"));
    assert!(!attributes.contains_key("litellm.request.tools.declared"));
}

fn attributes_of_json(payload_json: &str) -> AttributeMap {
    let payload: Payload = serde_json::from_str(payload_json).expect("payload");
    let call = LLMCallSpanData::from_standard_logging_payload(&payload, LLMCallOptions::default())
        .expect("llm call");
    genai_mapper().map(&SpanData::LLMCall(&call), &BTreeMap::new())
}

#[test]
fn genai_failure_writes_the_error_class_as_error_type() {
    let attributes = attributes_of_json(
        r#"{"id": "call-1", "model": "gpt-4o", "status": "failure",
            "error_information": {"error_class": "RateLimitError", "error_code": "429"}}"#,
    );
    assert_eq!(
        attributes.get("error.type"),
        Some(&Value::from("RateLimitError"))
    );
    let without_class = attributes_of_json(
        r#"{"id": "call-1", "model": "gpt-4o", "status": "failure",
            "error_information": {"error_message": "boom"}}"#,
    );
    assert!(!without_class.contains_key("error.type"));
}

#[rstest::rstest]
#[case("original", 0.5_f64)]
#[case("discount_amount", 0.25_f64)]
#[case("discount_percent", 0.125_f64)]
#[case("margin_fixed_amount", 0.75_f64)]
#[case("margin_percent", 0.375_f64)]
#[case("margin_total_amount", 0.625_f64)]
fn genai_llm_call_writes_the_cost_breakdown_component(#[case] key: &str, #[case] expected: f64) {
    let attributes = attributes_of_json(
        r#"{"id": "call-1", "model": "gpt-4o", "cost_breakdown": {
            "original_cost": 0.5, "discount_amount": 0.25, "discount_percent": 0.125,
            "margin_fixed_amount": 0.75, "margin_percent": 0.375, "margin_total_amount": 0.625}}"#,
    );
    assert_eq!(
        attributes.get(&format!("litellm.cost.{key}")),
        Some(&Value::F64(expected))
    );
}

#[test]
fn genai_llm_call_writes_the_cache_token_counts() {
    let attributes = attributes_of_json(
        r#"{"id": "call-1", "model": "gpt-4o", "prompt_tokens": 20, "metadata": {"usage_object": {
            "cache_creation_input_tokens": 3, "prompt_tokens_details": {"cached_tokens": 5}}}}"#,
    );
    assert_eq!(
        attributes.get("gen_ai.usage.cache_creation.input_tokens"),
        Some(&Value::I64(3))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.cache_read.input_tokens"),
        Some(&Value::I64(5))
    );
}

#[test]
fn genai_llm_call_writes_the_output_type_of_media_calls() {
    let image = attributes_of_json(
        r#"{"id": "call-1", "model": "dall-e-3", "call_type": "image_generation"}"#,
    );
    assert_eq!(image.get("gen_ai.output.type"), Some(&Value::from("image")));
    let speech = attributes_of_json(r#"{"id": "call-1", "model": "tts-1", "call_type": "speech"}"#);
    assert_eq!(
        speech.get("gen_ai.output.type"),
        Some(&Value::from("speech"))
    );
    let chat =
        attributes_of_json(r#"{"id": "call-1", "model": "gpt-4o", "call_type": "completion"}"#);
    assert!(!chat.contains_key("gen_ai.output.type"));
}

#[test]
fn genai_llm_call_writes_the_server_address_and_port() {
    let attributes = attributes_of_json(
        r#"{"id": "call-1", "model": "gpt-4o", "api_base": "https://llm.example:8443/v1"}"#,
    );
    assert_eq!(
        attributes.get("server.address"),
        Some(&Value::from("llm.example"))
    );
    assert_eq!(attributes.get("server.port"), Some(&Value::I64(8443)));
}

#[test]
fn genai_mcp_tool_call_without_a_server_omits_the_rpc_system() {
    let payload: Payload = serde_json::from_str(
        r#"{"id": "call-1", "call_type": "call_mcp_tool", "metadata": {
            "mcp_tool_call_metadata": {"name": "get_weather", "mcp_server_name": "weather"}}}"#,
    )
    .expect("payload");
    let tool_call =
        MCPToolCallSpanData::from_standard_logging_payload(&payload, true).expect("tool call");
    let attributes = genai_mapper().map(&SpanData::MCPToolCall(&tool_call), &BTreeMap::new());
    assert!(!attributes.contains_key("rpc.system"));
    assert!(!attributes.contains_key("server.port"));
    assert_eq!(
        attributes.get("gen_ai.tool.name"),
        Some(&Value::from("get_weather"))
    );
}
