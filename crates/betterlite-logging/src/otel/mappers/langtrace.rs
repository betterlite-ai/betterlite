use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::collect;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (gen_ai::OPERATION_NAME, |_data| Some(Value::from("chat"))),
    ("langtrace.service.name", |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    ("llm.model", |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    (gen_ai::RESPONSE_MODEL, |data| data.response_model.clone().map(Value::from)),
    ("gen_ai.response_id", |data| data.response_id.clone().map(Value::from)),
    ("gen_ai.system_fingerprint", |data| data.system_fingerprint.clone().map(Value::from)),
    ("llm.temperature", |data| data.request_params.temperature.map(Value::F64)),
    ("llm.top_p", |data| data.request_params.top_p.map(Value::F64)),
    ("llm.top_k", |data| data.request_params.top_k.map(Value::I64)),
    ("llm.max_tokens", |data| data.request_params.max_tokens.map(Value::I64)),
    ("llm.frequency_penalty", |data| data.request_params.frequency_penalty.map(Value::F64)),
    ("llm.presence_penalty", |data| data.request_params.presence_penalty.map(Value::F64)),
    ("llm.stream", |data| data.is_streaming.map(Value::Bool)),
    ("llm.token.counts.prompt", |data| data.usage.input_tokens.map(Value::I64)),
    ("llm.token.counts.completion", |data| data.usage.output_tokens.map(Value::I64)),
    ("llm.token.counts.total", |data| data.usage.total_tokens.map(Value::I64)),
];

#[rustfmt::skip]
const BLOB_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("llm.prompts", |data| {
        let text = (!data.messages_in.is_empty()).then(|| serde_json::to_string(&data.messages_in)).and_then(Result::ok);
        text.map(Value::from)
    }),
    ("llm.completions", |data| completion_blob(data).map(Value::from)),
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
