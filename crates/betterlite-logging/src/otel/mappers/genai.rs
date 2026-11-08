use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, Extractor, SpanData};
use super::utils::{
    collect, decimal_attr, output_messages, serialize_messages, string_array, tool_definition_attrs,
};
use crate::otel::model::db_endpoint::db_span_attributes;
use crate::otel::model::payloads::guardrail::GuardrailSpanData;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::mcp_call::{MCPListToolsSpanData, MCPToolCallSpanData};
use crate::otel::model::payloads::service::ServiceSpanData;
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::vocabulary::constants::RpcSystem;
use crate::otel::vocabulary::constants::attribute_keys::{
    error, gen_ai, json_rpc, litellm, mcp, server,
};

const TOOL_KEY_PREFIX: &str = "gen_ai.tool.";

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (gen_ai::OPERATION_NAME, |data| Some(Value::from(data.operation.to_string()))),
    (gen_ai::PROVIDER_NAME, |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    (gen_ai::OUTPUT_TYPE, |data| data.output_type.map(|kind| Value::from(kind.to_string()))),
    (gen_ai::CONVERSATION_ID, |data| data.session_id.clone().map(Value::from)),
    (gen_ai::REQUEST_MODEL, |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    (gen_ai::REQUEST_TEMPERATURE, |data| data.request_params.temperature.map(Value::F64)),
    (gen_ai::REQUEST_TOP_P, |data| data.request_params.top_p.map(Value::F64)),
    (gen_ai::REQUEST_TOP_K, |data| data.request_params.top_k.map(Value::I64)),
    (gen_ai::REQUEST_MAX_TOKENS, |data| data.request_params.max_tokens.map(Value::I64)),
    (gen_ai::REQUEST_FREQUENCY_PENALTY, |data| data.request_params.frequency_penalty.map(Value::F64)),
    (gen_ai::REQUEST_PRESENCE_PENALTY, |data| data.request_params.presence_penalty.map(Value::F64)),
    (gen_ai::REQUEST_STOP_SEQUENCES, |data| data.request_params.stop_sequences.as_ref().filter(|sequences| !sequences.is_empty()).map(|sequences| string_array(sequences))),
    (gen_ai::REQUEST_SEED, |data| data.request_params.seed.map(Value::I64)),
    (gen_ai::INPUT_MESSAGES, |data| serialize_messages(&data.messages_in).map(Value::from)),
    (gen_ai::OUTPUT_MESSAGES, |data| serialize_messages(&output_messages(data)).map(Value::from)),
    (gen_ai::RESPONSE_MODEL, |data| data.response_model.clone().map(Value::from)),
    (gen_ai::RESPONSE_ID, |data| data.response_id.clone().map(Value::from)),
    (gen_ai::RESPONSE_FINISH_REASONS, |data| (!data.finish_reasons.is_empty()).then(|| string_array(&data.finish_reasons))),
    (gen_ai::RESPONSE_TIME_TO_FIRST_CHUNK, |data| decimal_attr(data.time_to_first_chunk_seconds)),
    (gen_ai::USAGE_INPUT_TOKENS, |data| data.usage.input_tokens.map(Value::I64)),
    (gen_ai::USAGE_OUTPUT_TOKENS, |data| data.usage.output_tokens.map(Value::I64)),
    (gen_ai::USAGE_CACHE_CREATION_TOKENS, |data| data.usage.cache_creation_input_tokens.map(Value::I64)),
    (gen_ai::USAGE_CACHE_READ_INPUT_TOKENS, |data| data.usage.cache_read_input_tokens.map(Value::I64)),
    (error::TYPE, |data| data.error.as_ref().and_then(|failure| failure.error_type.clone()).map(Value::from)),
    (server::ADDRESS, |data| data.server.as_ref().and_then(|endpoint| endpoint.address.clone()).map(Value::from)),
    (server::PORT, |data| data.server.as_ref().and_then(|endpoint| endpoint.port).map(|port| Value::I64(i64::from(port)))),
    (litellm::CALL_ID, |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
    (litellm::CALL_TYPE, |data| data.call_type.clone().map(Value::from)),
    (litellm::PROVIDER_MODEL, |data| data.identity.provider_model.as_deref().filter(|model| !model.is_empty()).map(|model| Value::from(model.to_owned()))),
    (litellm::REQUEST_STREAMING, |data| data.is_streaming.map(Value::Bool)),
    (litellm::REQUEST_ROUTE, |data| data.request_route.clone().map(Value::from)),
];

#[rustfmt::skip]
const COST_ATTRS: &[(&str, Extractor<LLMCallSpanData>)] = &[
    ("total", |data| decimal_attr(data.response_cost)),
    ("input", |data| decimal_attr(data.cost.input)),
    ("output", |data| decimal_attr(data.cost.output)),
    ("cache_read", |data| decimal_attr(data.cost.cache_read)),
    ("cache_creation", |data| decimal_attr(data.cost.cache_creation)),
    ("tool_usage", |data| decimal_attr(data.cost.tool_usage)),
    ("original", |data| decimal_attr(data.cost.original)),
    ("discount_amount", |data| decimal_attr(data.cost.discount_amount)),
    ("discount_percent", |data| decimal_attr(data.cost.discount_percent)),
    ("margin_fixed_amount", |data| decimal_attr(data.cost.margin_fixed_amount)),
    ("margin_percent", |data| decimal_attr(data.cost.margin_percent)),
    ("margin_total_amount", |data| decimal_attr(data.cost.margin_total_amount)),
];

#[rustfmt::skip]
const TOOL_ATTRS: AttributeTable<ToolDefinition> = &[
    ("name", |tool| Some(Value::from(tool.name.clone()))),
    ("description", |tool| tool.description.clone().filter(|description| !description.is_empty()).map(Value::from)),
    ("parameters", |tool| tool.parameters_json.clone().filter(|parameters| !parameters.is_empty()).map(Value::from)),
];

#[rustfmt::skip]
const MCP_ATTRS: AttributeTable<MCPToolCallSpanData> = &[
    (gen_ai::OPERATION_NAME, |data| Some(Value::from(data.operation.to_string()))),
    (json_rpc::SYSTEM, |data| (data.server_address.as_deref().is_some_and(|address| !address.is_empty()) && data.server_port.is_some_and(|port| port != 0)).then(|| Value::from(RpcSystem::JsonRpc.to_string()))),
    (mcp::METHOD_NAME, |data| Some(Value::from(data.method.to_string()))),
    (mcp::SESSION_ID, |data| data.session_id.clone().map(Value::from)),
    (gen_ai::TOOL_NAME, |data| (!data.tool_name.is_empty()).then(|| Value::from(data.tool_name.clone()))),
    (gen_ai::TOOL_CALL_ARGUMENTS, |data| data.arguments_json.clone().map(Value::from)),
    (gen_ai::TOOL_CALL_RESULT, |data| data.result_json.clone().map(Value::from)),
    (server::ADDRESS, |data| data.server_address.clone().map(Value::from)),
    (server::PORT, |data| data.server_port.map(|port| Value::I64(i64::from(port)))),
    (litellm::MCP_SERVER_NAME, |data| data.server_name.clone().map(Value::from)),
    (litellm::CALL_ID, |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
];

#[rustfmt::skip]
const MCP_LIST_ATTRS: AttributeTable<MCPListToolsSpanData> = &[
    (mcp::METHOD_NAME, |data| Some(Value::from(data.method.to_string()))),
    (mcp::SESSION_ID, |data| data.session_id.clone().map(Value::from)),
    (litellm::CALL_ID, |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
];

#[rustfmt::skip]
const GUARDRAIL_ATTRS: AttributeTable<GuardrailSpanData> = &[
    (litellm::GUARDRAIL_NAME, |data| Some(Value::from(data.guardrail_name.clone()))),
    (litellm::GUARDRAIL_MODE, |data| data.mode.clone().map(Value::from)),
    (litellm::GUARDRAIL_STATUS, |data| data.status.clone().map(Value::from)),
    (litellm::GUARDRAIL_PROVIDER, |data| data.provider.clone().map(Value::from)),
    (litellm::GUARDRAIL_ACTION, |data| data.action.clone().map(Value::from)),
    (litellm::GUARDRAIL_RESPONSE, |data| data.response_json.clone().map(Value::from)),
    (litellm::GUARDRAIL_VIOLATION_CATEGORIES, |data| (!data.violation_categories.is_empty()).then(|| string_array(&data.violation_categories))),
    (litellm::GUARDRAIL_CONFIDENCE_SCORE, |data| data.confidence_score.map(Value::F64)),
    (litellm::GUARDRAIL_RISK_SCORE, |data| data.risk_score.map(Value::F64)),
    (litellm::GUARDRAIL_MASKED_ENTITY_COUNT, |data| data.masked_entity_count.map(Value::I64)),
    (litellm::GUARDRAIL_DURATION, |data| data.duration.map(Value::F64)),
    (litellm::GUARDRAIL_ID, |data| data.guardrail_id.clone().map(Value::from)),
    (litellm::GUARDRAIL_POLICY_TEMPLATE, |data| data.policy_template.clone().map(Value::from)),
    (litellm::GUARDRAIL_DETECTION_METHOD, |data| data.detection_method.clone().map(Value::from)),
    (litellm::GUARDRAIL_USAGE, |data| data.usage_json.clone().map(Value::from)),
    (litellm::GUARDRAIL_COST, |data| decimal_attr(data.cost)),
    (litellm::GUARDRAIL_COST_IN_SPEND, |data| data.cost_in_spend.map(Value::Bool)),
];

#[rustfmt::skip]
const SERVICE_ATTRS: AttributeTable<ServiceSpanData> = &[
    (litellm::SERVICE_NAME, |data| Some(Value::from(data.service_name.clone()))),
    (litellm::SERVICE_CALL_TYPE, |data| data.call_type.clone().map(Value::from)),
    (litellm::SERVICE_CALLER, |data| data.caller.clone().map(Value::from)),
];

pub struct GenAIMapper {
    pub tool_attr_budget: usize,
}

impl AttributeMapper for GenAIMapper {
    fn map(&self, data: &SpanData<'_>, environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => self.llm_call(call),
            SpanData::MCPToolCall(call) => tool_call_attributes(call),
            SpanData::MCPListTools(call) => collect(MCP_LIST_ATTRS, call),
            SpanData::Guardrail(guardrail) => collect(GUARDRAIL_ATTRS, guardrail),
            SpanData::Service(service) => service_attributes(service, environment),
        }
    }
}

impl GenAIMapper {
    fn llm_call(&self, data: &LLMCallSpanData) -> AttributeMap {
        let mut attrs = collect(LLM_CALL_ATTRS, data);
        for (suffix, extract) in COST_ATTRS {
            if let Some(value) = (*extract)(data) {
                attrs.insert(format!("{}{suffix}", litellm::COST_PREFIX), value);
            }
        }
        if data.tools.is_empty() {
            return attrs;
        }
        if let Ok(declared) = i64::try_from(data.tools.len()) {
            attrs.insert(litellm::TOOLS_DECLARED.to_owned(), Value::I64(declared));
        }
        attrs.extend(tool_definition_attrs(
            TOOL_KEY_PREFIX,
            &data.tools,
            TOOL_ATTRS,
            self.tool_attr_budget,
        ));
        attrs
    }
}

fn tool_call_attributes(data: &MCPToolCallSpanData) -> AttributeMap {
    let mut attrs = collect(MCP_ATTRS, data);
    if let Some(value) = decimal_attr(data.response_cost) {
        attrs.insert(format!("{}total", litellm::COST_PREFIX), value);
    }
    attrs
}

fn service_attributes(data: &ServiceSpanData, environment: &dyn EnvironmentLookup) -> AttributeMap {
    let mut attrs = collect(SERVICE_ATTRS, data);
    let call_type = data.call_type.as_deref();
    for pair in db_span_attributes(&data.service_name, call_type, environment) {
        attrs.insert(pair.key.as_str().to_owned(), pair.value);
    }
    for (key, value) in &data.event_metadata {
        attrs.insert(
            format!("{}{key}", litellm::METADATA_PREFIX),
            Value::from(value.clone()),
        );
    }
    attrs
}

#[path = "genai/otel_mappers/mod.rs"]
mod otel_mappers;
