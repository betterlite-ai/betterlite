use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;
use rust_decimal::Decimal;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::{collect, output_messages, serialize_messages, string_array};
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::trace_controls::TraceControls;

#[derive(serde::Serialize)]
struct ModelParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(serde::Serialize)]
struct UsageDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_cached_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_cache_creation: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_reasoning_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<i64>,
}

#[derive(serde::Serialize)]
struct CostDetails {
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    total: Decimal,
}

// one attribute per line keeps a table readable; rustfmt stacks every closure vertically
#[rustfmt::skip]
const LLM_CALL_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("langfuse.observation.type", |_data| Some(Value::from("generation"))),
    ("langfuse.observation.model.name", |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    ("langfuse.observation.metadata.provider", |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    ("langfuse.observation.id", |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
    ("langfuse.trace.metadata.team_id", |data| data.identity.team_id.as_deref().filter(|team_id| !team_id.is_empty()).map(|team_id| Value::from(team_id.to_owned()))),
    ("langfuse.trace.metadata.team_alias", |data| data.identity.team_alias.as_deref().filter(|alias| !alias.is_empty()).map(|alias| Value::from(alias.to_owned()))),
];

#[rustfmt::skip]
const BLOB_ATTRS: AttributeTable<LLMCallSpanData> = &[
    ("langfuse.observation.model.parameters", |data| {
        let parameters = ModelParameters {
            temperature: data.request_params.temperature,
            top_p: data.request_params.top_p,
            max_tokens: data.request_params.max_tokens,
            frequency_penalty: data.request_params.frequency_penalty,
            presence_penalty: data.request_params.presence_penalty,
            seed: data.request_params.seed,
        };
        serde_json::to_string(&parameters).ok().filter(|text| text != "{}").map(Value::from)
    }),
    ("langfuse.observation.input", |data| serialize_messages(&data.messages_in).map(Value::from)),
    ("langfuse.observation.output", |data| output_json(data).map(Value::from)),
    ("langfuse.observation.usage_details", |data| {
        let details = UsageDetails {
            input: data.usage.uncached_input_tokens(),
            input_cached_tokens: data.usage.cache_read_input_tokens,
            input_cache_creation: data.usage.cache_creation_input_tokens,
            output: data.usage.non_reasoning_output_tokens(),
            output_reasoning_tokens: data.usage.reasoning_tokens,
            total: data.usage.total_tokens,
        };
        serde_json::to_string(&details).ok().filter(|text| text != "{}").map(Value::from)
    }),
    ("langfuse.observation.cost_details", |data| data.response_cost.and_then(|cost| serde_json::to_string(&CostDetails { total: cost }).ok()).map(Value::from)),
];

pub struct LangfuseMapper;

impl AttributeMapper for LangfuseMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => {
                let mut attrs = collect(LLM_CALL_ATTRS, call);
                attrs.extend(collect(TRACE_ATTRS, &call.trace));
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

fn output_json(data: &LLMCallSpanData) -> Option<String> {
    data.embedding_output.as_ref().map_or_else(
        || serialize_messages(&output_messages(data)),
        |output| serde_json::to_string(output).ok(),
    )
}

#[rustfmt::skip]
const TRACE_ATTRS: AttributeTable<TraceControls> = &[
    ("langfuse.trace.name", |trace| trace.name.clone().filter(|name| !name.is_empty()).map(Value::from)),
    ("user.id", |trace| trace.user_id.clone().filter(|user_id| !user_id.is_empty()).map(Value::from)),
    ("session.id", |trace| trace.session_id.clone().filter(|session_id| !session_id.is_empty()).map(Value::from)),
    ("langfuse.trace.tags", |trace| (!trace.tags.is_empty()).then(|| string_array(&trace.tags))),
];

#[path = "langfuse/tests.rs"]
mod tests;
