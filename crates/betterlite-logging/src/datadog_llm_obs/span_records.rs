use rust_decimal::Decimal;
use serde::Serialize;

use super::records::{DdMessage, ToolDefinition};
use super::values::Object;

// The span kinds LLM Obs accepts on a Meta record.
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

// OutputMeta.messages carries the same projected records as the input side.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct OutputMeta {
    pub messages: Vec<DdMessage>,
}

// Error information on the span according to the DD LLM Obs API spec.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DDLLMObsError {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DDLLMObsError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_definitions: Vec<ToolDefinition>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LLMMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_per_output_token: Option<f64>,
    #[serde(
        with = "rust_decimal::serde::arbitrary_precision_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub total_cost: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_write_input_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_cached_input_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output_tokens: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LLMObsPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ns: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<LLMMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DDLLMObsLatencyMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub litellm_overhead_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail_overhead_time_ms: Option<f64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DDLLMObsSpendMetrics {
    #[serde(
        with = "rust_decimal::serde::arbitrary_precision_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub response_cost: Option<Decimal>,
    #[serde(
        with = "rust_decimal::serde::arbitrary_precision_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_api_key_spend: Option<Decimal>,
    #[serde(
        with = "rust_decimal::serde::arbitrary_precision_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_api_key_max_budget: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_api_key_budget_reset_at: Option<String>,
}
