use rust_decimal::Decimal;
use serde::Serialize;

use super::records::{DdMessage, ToolDefinition};
use super::values::Object;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanKind {
    Llm,
    Tool,
    Task,
    Embedding,
    Retrieval,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct InputMeta {
    pub messages: Vec<DdMessage>,
}

pub type OutputMeta = InputMeta;

// LiteLLM always writes message, stack and type, so an absent stack or error class goes out as null.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DDLLMObsError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub stack: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

// LiteLLM always writes meta.error, so a span without an error carries "error": null.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Meta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<SpanKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<InputMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Object>,
    pub error: Option<DDLLMObsError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_definitions: Vec<ToolDefinition>,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LLMMetrics {
    pub input_tokens: Option<f64>,
    pub output_tokens: Option<f64>,
    pub total_tokens: Option<f64>,
    pub time_to_first_token: Option<f64>,
    pub time_per_output_token: Option<f64>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub total_cost: Option<Decimal>,
    pub cache_read_input_tokens: Option<f64>,
    pub cache_write_input_tokens: Option<f64>,
    pub non_cached_input_tokens: Option<f64>,
    pub reasoning_output_tokens: Option<f64>,
    pub tool_output_tokens: Option<f64>,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LLMObsPayload {
    pub parent_id: Option<String>,
    pub trace_id: Option<String>,
    pub apm_id: Option<String>,
    pub span_id: Option<String>,
    pub name: Option<String>,
    pub meta: Option<Meta>,
    pub start_ns: Option<i64>,
    pub duration: Option<i64>,
    pub metrics: Option<LLMMetrics>,
    pub tags: Option<Vec<String>>,
    pub status: Option<SpanStatus>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DDSpanAttributes {
    pub ml_app: String,
    pub tags: Vec<String>,
    pub spans: Vec<LLMObsPayload>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DDIntakePayload {
    #[serde(rename = "type")]
    pub kind: String,
    pub attributes: DDSpanAttributes,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DDLLMObsLatencyMetrics {
    pub time_to_first_token_ms: Option<f64>,
    pub litellm_overhead_time_ms: Option<f64>,
    pub guardrail_overhead_time_ms: Option<f64>,
}

#[serde_with::skip_serializing_none]
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DDLLMObsSpendMetrics {
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub response_cost: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_api_key_max_budget: Option<Decimal>,
    pub user_api_key_budget_reset_at: Option<String>,
}
