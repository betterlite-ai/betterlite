#![cfg(test)]

use opentelemetry::Value;
use std::collections::BTreeMap;

use crate::otel::mappers::base::{AttributeMap, AttributeMapper, SpanData};
use crate::otel::mappers::genai::GenAIMapper;
use crate::otel::mappers::legacy::LegacyMapper;
use crate::otel::mappers::utils::MAX_TOOL_ATTRS_PER_SPAN;
use crate::otel::model::payloads::RequestIdentity;
use crate::otel::model::payloads::cost::LLMCost;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::request_params::LLMRequestParams;
use crate::otel::model::payloads::usage::LLMUsage;
use crate::otel::model::trace_controls::TraceControls;
use crate::otel::vocabulary::constants::GenAIOperation;

fn llm_call() -> LLMCallSpanData {
    LLMCallSpanData {
        operation: GenAIOperation::Chat,
        provider: "openai".to_owned(),
        request_model: "gpt-4o".to_owned(),
        response_model: None,
        response_id: None,
        request_params: LLMRequestParams::default(),
        usage: LLMUsage::default(),
        finish_reasons: Vec::new(),
        error: None,
        response_cost: None,
        server: None,
        identity: RequestIdentity::default(),
        is_streaming: None,
        cost: LLMCost::default(),
        tools: Vec::new(),
        messages_in: Vec::new(),
        choices_out: Vec::new(),
        system_fingerprint: None,
        time_to_first_chunk_seconds: None,
        output_type: None,
        call_type: None,
        request_route: None,
        trace: TraceControls::default(),
        session_id: None,
        embedding_output: None,
    }
}

#[test]
fn mapper_protocol() {
    let call = llm_call();
    let environment = BTreeMap::new();
    let genai = GenAIMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let legacy = LegacyMapper {
        tool_attr_budget: MAX_TOOL_ATTRS_PER_SPAN,
    };
    let mappers: [&dyn AttributeMapper; 2] = [&genai, &legacy];
    let mapped = mappers[0].map(&SpanData::LLMCall(&call), &environment);
    assert_eq!(
        mapped.get("gen_ai.operation.name"),
        Some(&Value::from("chat"))
    );
    assert!(
        !mappers[1]
            .map(&SpanData::LLMCall(&call), &environment)
            .is_empty()
    );
    let guardrail = GuardrailSpanData::from_logging_entry(&BTreeMap::new());
    assert_eq!(
        mappers[1].map(&SpanData::Guardrail(&guardrail), &environment),
        AttributeMap::new()
    );
}
