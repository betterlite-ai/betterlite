use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::collect;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;

const SERVICE_NAME: &str = "langtrace.service.name";
const MODEL: &str = "llm.model";
const RESPONSE_ID: &str = "gen_ai.response_id";
const SYSTEM_FINGERPRINT: &str = "gen_ai.system_fingerprint";
const TEMPERATURE: &str = "llm.temperature";
const TOP_P: &str = "llm.top_p";
const TOP_K: &str = "llm.top_k";
const MAX_TOKENS: &str = "llm.max_tokens";
const FREQUENCY_PENALTY: &str = "llm.frequency_penalty";
const PRESENCE_PENALTY: &str = "llm.presence_penalty";
const STREAM: &str = "llm.stream";
const PROMPT_TOKENS: &str = "llm.token.counts.prompt";
const COMPLETION_TOKENS: &str = "llm.token.counts.completion";
const TOTAL_TOKENS: &str = "llm.token.counts.total";
const PROMPTS: &str = "llm.prompts";
const COMPLETIONS: &str = "llm.completions";

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (gen_ai::OPERATION_NAME, |_data| Some(Value::from("chat"))),
    (SERVICE_NAME, |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    (MODEL, |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    (gen_ai::RESPONSE_MODEL, |data| data.response_model.clone().map(Value::from)),
    (RESPONSE_ID, |data| data.response_id.clone().map(Value::from)),
    (SYSTEM_FINGERPRINT, |data| data.system_fingerprint.clone().map(Value::from)),
    (TEMPERATURE, |data| data.request_params.temperature.map(Value::F64)),
    (TOP_P, |data| data.request_params.top_p.map(Value::F64)),
    (TOP_K, |data| data.request_params.top_k.map(Value::I64)),
    (MAX_TOKENS, |data| data.request_params.max_tokens.map(Value::I64)),
    (FREQUENCY_PENALTY, |data| data.request_params.frequency_penalty.map(Value::F64)),
    (PRESENCE_PENALTY, |data| data.request_params.presence_penalty.map(Value::F64)),
    (STREAM, |data| data.is_streaming.map(Value::Bool)),
    (PROMPT_TOKENS, |data| data.usage.input_tokens.map(Value::I64)),
    (COMPLETION_TOKENS, |data| data.usage.output_tokens.map(Value::I64)),
    (TOTAL_TOKENS, |data| data.usage.total_tokens.map(Value::I64)),
];

#[rustfmt::skip]
const BLOB_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (PROMPTS, |data| {
        let text = (!data.messages_in.is_empty()).then(|| serde_json::to_string(&data.messages_in)).and_then(Result::ok);
        text.map(Value::from)
    }),
    (COMPLETIONS, |data| completion_blob(data).map(Value::from)),
];

// LiteLLM keeps one entry per choice and writes null for a choice without a message
fn completion_blob(data: &LLMCallSpanData) -> Option<String> {
    if data.choices_out.is_empty() {
        return None;
    }
    let messages: Vec<Option<&MetadataMap>> = data
        .choices_out
        .iter()
        .map(|choice| choice.get("message").and_then(OtelModelValue::as_object))
        .collect();
    serde_json::to_string(&messages).ok()
}

pub struct LangtraceMapper;

impl AttributeMapper for LangtraceMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => {
                let mut attrs = collect(LLM_CALL_ATTRS, call);
                attrs.extend(collect(BLOB_ATTRS, call));
                attrs
            }
            SpanData::MCPToolCall(_)
            | SpanData::MCPListTools(_)
            | SpanData::Guardrail(_)
            | SpanData::Service(_) => AttributeMap::new(),
        }
    }
}

#[path = "langtrace/tests.rs"]
mod tests;
