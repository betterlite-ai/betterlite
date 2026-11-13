#![cfg(test)]

use opentelemetry::{Array, StringValue, Value};
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMapper as _, SpanData};
use crate::otel::mappers::legacy::LegacyMapper;
use crate::otel::mappers::utils::MAX_TOOL_ATTRS_PER_SPAN;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::{LLMCallOptions, LLMCallSpanData};
use crate::otel::model::payloads::mcp_call::MCPToolCallSpanData;
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData, ServiceType};
use crate::otel::model::utils::coercion::OtelModelValue;

const PAYLOAD: &str = r#"{
    "id": "call-1",
    "model": "gpt-4o",
    "prompt_tokens": 10,
    "completion_tokens": 4,
    "total_tokens": 14,
    "custom_llm_provider": "anthropic",
    "call_type": "completion",
    "stream": true,
    "model_parameters": {
        "top_k": 5,
        "frequency_penalty": 0.1,
        "presence_penalty": 0.2,
        "stop": ["END"],
        "tools": [
            {"type": "function", "function": {"name": "get_weather", "description": "Weather", "parameters": {"type": "object"}}}
        ]
    }
}"#;

fn legacy_mapper() -> LegacyMapper {
    LegacyMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    }
}

fn legacy_call() -> LLMCallSpanData {
    let payload: Payload = serde_json::from_str(PAYLOAD).expect("payload");
    LLMCallSpanData::from_standard_logging_payload(&payload, LLMCallOptions::default())
        .expect("llm call")
}

#[test]
fn legacy_llm_call_uses_semconv_ai_names() {
    let call = legacy_call();
    let attributes = legacy_mapper().map(&SpanData::LLMCall(&call), &BTreeMap::new());
    assert_eq!(
        attributes.get("gen_ai.system"),
        Some(&Value::from("anthropic"))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.prompt_tokens"),
        Some(&Value::I64(10))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.completion_tokens"),
        Some(&Value::I64(4))
    );
    assert_eq!(
        attributes.get("gen_ai.usage.total_tokens"),
        Some(&Value::I64(14))
    );
    assert_eq!(attributes.get("llm.is_streaming"), Some(&Value::Bool(true)));
    assert_eq!(attributes.get("llm.top_k"), Some(&Value::I64(5)));
    assert_eq!(
        attributes.get("llm.chat.stop_sequences"),
        Some(&Value::Array(Array::String(vec![StringValue::from(
            "END".to_owned()
        )])))
    );
    assert_eq!(
        attributes.get("llm.request.functions.0.name"),
        Some(&Value::from("get_weather"))
    );
    assert_eq!(
        attributes.get("llm.request.functions.0.description"),
        Some(&Value::from("Weather"))
    );
    assert!(!attributes.contains_key("gen_ai.request.model"));
    assert!(!attributes.contains_key("gen_ai.provider.name"));
}

#[test]
fn legacy_service_uses_bare_keys() {
    let service = ServiceSpanData::from_payload(
        &ServiceLoggerPayload {
            service: ServiceType::Redis,
            call_type: "redis_get".to_owned(),
            caller: Some("litellm.completion".to_owned()),
            error: Some("connection refused".to_owned()),
        },
        &BTreeMap::from([(
            "operation".to_owned(),
            OtelModelValue::String("get".to_owned()),
        )]),
    );
    let attributes = legacy_mapper().map(&SpanData::Service(&service), &BTreeMap::new());
    assert_eq!(attributes.get("service"), Some(&Value::from("redis")));
    assert_eq!(attributes.get("call_type"), Some(&Value::from("redis_get")));
    assert_eq!(
        attributes.get("caller"),
        Some(&Value::from("litellm.completion"))
    );
    assert_eq!(
        attributes.get("error"),
        Some(&Value::from("connection refused"))
    );
    assert_eq!(attributes.get("operation"), Some(&Value::from("get")));
    assert!(!attributes.contains_key("litellm.metadata.operation"));
}

#[test]
fn legacy_mapper_returns_nothing_for_unspoken_spans() {
    let payload = Payload::default();
    let tool_call =
        MCPToolCallSpanData::from_standard_logging_payload(&payload, false).expect("tool call");
    let guardrail = GuardrailSpanData::from_logging_entry(&BTreeMap::new());
    let mapper = legacy_mapper();
    let environment = BTreeMap::new();
    assert!(
        mapper
            .map(&SpanData::MCPToolCall(&tool_call), &environment)
            .is_empty()
    );
    assert!(
        mapper
            .map(&SpanData::Guardrail(&guardrail), &environment)
            .is_empty()
    );
}
