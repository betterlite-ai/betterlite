use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::{collect, string_array, tool_definition_attrs};
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::payloads::service::ServiceSpanData;
use crate::otel::model::payloads::tool_definition::ToolDefinition;
use crate::otel::vocabulary::constants::attribute_keys::{error, gen_ai};

const LEGACY_PROMPT_TOKENS: &str = "gen_ai.usage.prompt_tokens";
const LEGACY_COMPLETION_TOKENS: &str = "gen_ai.usage.completion_tokens";
const LEGACY_TOTAL_TOKENS: &str = "gen_ai.usage.total_tokens";
const LEGACY_IS_STREAMING: &str = "llm.is_streaming";
const LEGACY_TOP_K: &str = "llm.top_k";
const LEGACY_FREQUENCY_PENALTY: &str = "llm.frequency_penalty";
const LEGACY_PRESENCE_PENALTY: &str = "llm.presence_penalty";
const LEGACY_STOP_SEQUENCES: &str = "llm.chat.stop_sequences";
const LEGACY_SERVICE: &str = "service";
const LEGACY_CALL_TYPE: &str = "call_type";
const LEGACY_CALLER: &str = "caller";
const TOOL_KEY_PREFIX: &str = "llm.request.functions.";

#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (gen_ai::SYSTEM, |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    (LEGACY_PROMPT_TOKENS, |data| data.usage.input_tokens.map(Value::I64)),
    (LEGACY_COMPLETION_TOKENS, |data| data.usage.output_tokens.map(Value::I64)),
    (LEGACY_TOTAL_TOKENS, |data| data.usage.total_tokens.map(Value::I64)),
    (LEGACY_IS_STREAMING, |data| data.is_streaming.map(Value::Bool)),
    (LEGACY_TOP_K, |data| data.request_params.top_k.map(Value::I64)),
    (LEGACY_FREQUENCY_PENALTY, |data| data.request_params.frequency_penalty.map(Value::F64)),
    (LEGACY_PRESENCE_PENALTY, |data| data.request_params.presence_penalty.map(Value::F64)),
    (LEGACY_STOP_SEQUENCES, |data| data.request_params.stop_sequences.as_ref().filter(|sequences| !sequences.is_empty()).map(|sequences| string_array(sequences))),
];

#[rustfmt::skip]
const TOOL_ATTRS: AttributeTable<ToolDefinition> = &[
    ("name", |tool| Some(Value::from(tool.name.clone()))),
    ("description", |tool| tool.description.clone().filter(|description| !description.is_empty()).map(Value::from)),
    ("parameters", |tool| tool.parameters_json.clone().filter(|parameters| !parameters.is_empty()).map(Value::from)),
];

#[rustfmt::skip]
const SERVICE_ATTRS: AttributeTable<ServiceSpanData> = &[
    (LEGACY_SERVICE, |data| Some(Value::from(data.service_name.clone()))),
    (LEGACY_CALL_TYPE, |data| data.call_type.clone().map(Value::from)),
    (LEGACY_CALLER, |data| data.caller.clone().map(Value::from)),
    (error::MESSAGE_LEGACY, |data| data.error.as_ref().and_then(|failure| failure.message.clone()).filter(|message| !message.is_empty()).map(Value::from)),
];

pub struct LegacyMapper {
    pub tool_attr_budget: usize,
}

impl AttributeMapper for LegacyMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => self.llm_call(call),
            SpanData::Service(service) => service_attributes(service),
            SpanData::MCPToolCall(_) | SpanData::MCPListTools(_) | SpanData::Guardrail(_) => {
                AttributeMap::new()
            }
        }
    }
}

impl LegacyMapper {
    fn llm_call(&self, data: &LLMCallSpanData) -> AttributeMap {
        let mut attrs = collect(LLM_CALL_ATTRS, data);
        attrs.extend(tool_definition_attrs(
            TOOL_KEY_PREFIX,
            &data.tools,
            TOOL_ATTRS,
            self.tool_attr_budget,
        ));
        attrs
    }
}

fn service_attributes(data: &ServiceSpanData) -> AttributeMap {
    let mut attrs = collect(SERVICE_ATTRS, data);
    for (key, value) in &data.event_metadata {
        attrs.insert(key.clone(), Value::from(value.clone()));
    }
    attrs
}

#[path = "legacy/otel_mappers/mod.rs"]
mod otel_mappers;
