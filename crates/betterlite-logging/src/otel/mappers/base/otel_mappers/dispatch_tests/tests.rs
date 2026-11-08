#![cfg(test)]

use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMapper as _, SpanData};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::legacy::LegacyMapper;
use crate::otel::mappers::utils::MAX_TOOL_ATTRS_PER_SPAN;
use crate::otel::model::metadata::wire::payload::Payload;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::mcp_call::{MCPListToolsSpanData, MCPToolCallSpanData};
use crate::otel::model::payloads::service::{ServiceLoggerPayload, ServiceSpanData, ServiceType};

fn genai_mapper() -> GenAIMapper {
    GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    }
}

fn legacy_mapper() -> LegacyMapper {
    LegacyMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    }
}

fn service_span() -> ServiceSpanData {
    ServiceSpanData::from_payload(
        &ServiceLoggerPayload {
            service: ServiceType::Redis,
            call_type: "redis_get".to_owned(),
            caller: Some("litellm.completion".to_owned()),
            error: None,
        },
        &BTreeMap::new(),
    )
}

#[test]
fn genai_mapper_answers_every_span_kind() {
    let payload = Payload::default();
    let environment = BTreeMap::new();
    let mapper = genai_mapper();
    let tool_call = MCPToolCallSpanData::from_standard_logging_payload(&payload, false)
        .expect("tool call span");
    let list_tools =
        MCPListToolsSpanData::from_standard_logging_payload(&payload).expect("list tools span");
    let guardrail = GuardrailSpanData::from_logging_entry(&BTreeMap::new());
    assert!(
        mapper
            .map(&SpanData::MCPToolCall(&tool_call), &environment)
            .contains_key("gen_ai.operation.name")
    );
    assert!(
        mapper
            .map(&SpanData::MCPListTools(&list_tools), &environment)
            .contains_key("mcp.method.name")
    );
    assert!(
        mapper
            .map(&SpanData::Guardrail(&guardrail), &environment)
            .contains_key("litellm.guardrail.name")
    );
    assert!(
        mapper
            .map(&SpanData::Service(&service_span()), &environment)
            .contains_key("litellm.service.name")
    );
}

#[test]
fn legacy_mapper_answers_only_llm_calls_and_services() {
    let service = service_span();
    let mapper = legacy_mapper();
    let environment = BTreeMap::new();
    let payload = Payload::default();
    let tool_call = MCPToolCallSpanData::from_standard_logging_payload(&payload, false)
        .expect("tool call span");
    let list_tools =
        MCPListToolsSpanData::from_standard_logging_payload(&payload).expect("list tools span");
    let guardrail = GuardrailSpanData::from_logging_entry(&BTreeMap::new());
    assert!(
        mapper
            .map(&SpanData::Service(&service), &environment)
            .contains_key("service")
    );
    assert!(
        mapper
            .map(&SpanData::MCPToolCall(&tool_call), &environment)
            .is_empty()
    );
    assert!(
        mapper
            .map(&SpanData::MCPListTools(&list_tools), &environment)
            .is_empty()
    );
    assert!(
        mapper
            .map(&SpanData::Guardrail(&guardrail), &environment)
            .is_empty()
    );
}
