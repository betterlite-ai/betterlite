use betterlite_config::loader::EnvironmentLookup;
use opentelemetry::Value;
use rust_decimal::Decimal;

use super::base::{AttributeMap, AttributeMapper, AttributeTable, SpanData};
use super::utils::{collect, output_messages, serialize_messages, string_array};
use crate::otel::model::payloads::llm_call::LLMCallSpanData;
use crate::otel::model::trace_controls::TraceControls;

const OBSERVATION_TYPE: &str = "langfuse.observation.type";
const OBSERVATION_MODEL_NAME: &str = "langfuse.observation.model.name";
const OBSERVATION_PROVIDER: &str = "langfuse.observation.metadata.provider";
const OBSERVATION_ID: &str = "langfuse.observation.id";
const OBSERVATION_MODEL_PARAMETERS: &str = "langfuse.observation.model.parameters";
const OBSERVATION_INPUT: &str = "langfuse.observation.input";
const OBSERVATION_OUTPUT: &str = "langfuse.observation.output";
const OBSERVATION_USAGE_DETAILS: &str = "langfuse.observation.usage_details";
const OBSERVATION_COST_DETAILS: &str = "langfuse.observation.cost_details";
const TRACE_NAME: &str = "langfuse.trace.name";
const TRACE_USER_ID: &str = "user.id";
const TRACE_SESSION_ID: &str = "session.id";
const TRACE_TAGS: &str = "langfuse.trace.tags";
const TRACE_TEAM_ID: &str = "langfuse.trace.metadata.team_id";
const TRACE_TEAM_ALIAS: &str = "langfuse.trace.metadata.team_alias";

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
    (OBSERVATION_TYPE, |_data| Some(Value::from("generation"))),
    (OBSERVATION_MODEL_NAME, |data| (!data.request_model.is_empty()).then(|| Value::from(data.request_model.clone()))),
    (OBSERVATION_PROVIDER, |data| (!data.provider.is_empty()).then(|| Value::from(data.provider.clone()))),
    (OBSERVATION_ID, |data| data.identity.call_id.as_deref().filter(|call_id| !call_id.is_empty()).map(|call_id| Value::from(call_id.to_owned()))),
    (TRACE_TEAM_ID, |data| data.identity.team_id.as_deref().filter(|team_id| !team_id.is_empty()).map(|team_id| Value::from(team_id.to_owned()))),
    (TRACE_TEAM_ALIAS, |data| data.identity.team_alias.as_deref().filter(|alias| !alias.is_empty()).map(|alias| Value::from(alias.to_owned()))),
];

#[rustfmt::skip]
const BLOB_ATTRS: AttributeTable<LLMCallSpanData> = &[
    (OBSERVATION_MODEL_PARAMETERS, |data| {
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
    (OBSERVATION_INPUT, |data| serialize_messages(&data.messages_in).map(Value::from)),
    (OBSERVATION_OUTPUT, |data| output_json(data).map(Value::from)),
    (OBSERVATION_USAGE_DETAILS, |data| {
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
    (OBSERVATION_COST_DETAILS, |data| data.response_cost.and_then(|cost| serde_json::to_string(&CostDetails { total: cost }).ok()).map(Value::from)),
];

pub struct LangfuseMapper;

impl AttributeMapper for LangfuseMapper {
    fn map(&self, data: &SpanData<'_>, _environment: &dyn EnvironmentLookup) -> AttributeMap {
        match data {
            SpanData::LLMCall(call) => {
                let mut attrs = collect(LLM_CALL_ATTRS, call);
                attrs.extend(trace_attributes(&call.trace));
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

fn trace_attributes(trace: &TraceControls) -> AttributeMap {
    let mut attrs = AttributeMap::new();
    if let Some(name) = trace.name.as_deref().filter(|name| !name.is_empty()) {
        attrs.insert(TRACE_NAME.to_owned(), Value::from(name.to_owned()));
    }
    if let Some(user_id) = trace
        .user_id
        .as_deref()
        .filter(|user_id| !user_id.is_empty())
    {
        attrs.insert(TRACE_USER_ID.to_owned(), Value::from(user_id.to_owned()));
    }
    if let Some(session_id) = trace
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())
    {
        attrs.insert(
            TRACE_SESSION_ID.to_owned(),
            Value::from(session_id.to_owned()),
        );
    }
    if !trace.tags.is_empty() {
        attrs.insert(TRACE_TAGS.to_owned(), string_array(&trace.tags));
    }
    attrs
}

#[path = "langfuse/tests.rs"]
mod tests;
