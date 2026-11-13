use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

mod tests;

// litellm.integrations.deepeval.types.SpanApiType, as string values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanApiType {
    Base,
    Agent,
    Llm,
    Retriever,
    Tool,
}

// litellm.integrations.deepeval.types.TraceSpanApiStatus, as string values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TraceSpanApiStatus {
    Success,
    Errored,
}

// litellm.integrations.deepeval.types.Environment, validated by utils.validate_environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Production,
    Development,
    Staging,
}

// BaseApiSpan.input and .output are declared dict | list | str | None; null takes the builder's default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpanValue {
    Text(String),
    Object(IndexMap<String, Json>),
    List(Vec<Json>),
}

// The startTime and endTime strings _prepare_trace_api formats once and reuses for the span and the trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanTimes<'moment> {
    pub start: &'moment str,
    pub end: &'moment str,
}

// litellm.integrations.deepeval.types.BaseApiSpan with the fields DeepEvalLogger fills in; a None optional field is omitted, as exclude_none serializes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BaseApiSpan {
    pub uuid: String,
    pub name: String,
    pub status: TraceSpanApiStatus,
    #[serde(rename = "type")]
    pub span_type: SpanApiType,
    #[serde(rename = "traceUuid")]
    pub trace_uuid: String,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "endTime")]
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<SpanValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<SpanValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(rename = "inputTokenCount", skip_serializing_if = "Option::is_none")]
    pub input_token_count: Option<i64>,
    #[serde(rename = "outputTokenCount", skip_serializing_if = "Option::is_none")]
    pub output_token_count: Option<i64>,
}

// litellm.integrations.deepeval.types.TraceApi: metadata and tags stay unset, so exclude_none drops them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TraceApi {
    pub uuid: String,
    #[serde(rename = "baseSpans")]
    pub base_spans: Vec<BaseApiSpan>,
    #[serde(rename = "agentSpans")]
    pub agent_spans: Vec<BaseApiSpan>,
    #[serde(rename = "llmSpans")]
    pub llm_spans: Vec<BaseApiSpan>,
    #[serde(rename = "retrieverSpans")]
    pub retriever_spans: Vec<BaseApiSpan>,
    #[serde(rename = "toolSpans")]
    pub tool_spans: Vec<BaseApiSpan>,
    #[serde(rename = "startTime")]
    pub start_time: String,
    #[serde(rename = "endTime")]
    pub end_time: String,
    pub environment: Environment,
}
