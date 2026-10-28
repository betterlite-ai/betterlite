use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::ui_content::UiContent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanType {
    Agent,
    Llm,
    Tool,
    Chain,
    Framework,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error,
    Unset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    #[serde(rename = "type")]
    pub span_type: SpanType,
    pub agent: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub start_offset_ms: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub duration_ms: Decimal,
    pub status: SpanStatus,
    pub error: Option<String>,
    pub error_truncated: bool,
    pub input_preview: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub litellm_request_id: Option<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentNode {
    pub name: String,
    pub parent_agent: Option<String>,
    pub invocations: i64,
    pub llm_calls: i64,
    pub tool_calls: i64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub duration_ms: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceSummary {
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_ref: Option<String>,
    pub name: String,
    pub service: String,
    pub input_preview: String,
    pub start_time: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub duration_ms: Decimal,
    pub status: SpanStatus,
    pub span_count: i64,
    pub agent_count: i64,
    pub agent_invocations: i64,
    pub llm_calls: i64,
    pub tool_calls: i64,
    pub error_count: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub models: Vec<String>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub spend: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    pub summary: TraceSummary,
    pub agents: Vec<AgentNode>,
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TracePage {
    pub data: Vec<TraceSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanDetail {
    pub span_id: String,
    pub input: String,
    pub output: String,
    pub input_ui: UiContent,
    pub output_ui: UiContent,
    pub attributes: IndexMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanErrorPage {
    pub span_id: String,
    pub message: String,
    pub total_chars: usize,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceScope {
    pub team_ids: Vec<String>,
    pub api_key_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLogRecord {
    pub request_id: String,
    pub response_id: String,
    pub call_type: String,
    pub api_key: String,
    pub key_alias: String,
    pub team_id: String,
    pub team_alias: String,
    pub organization_id: String,
    pub user: String,
    pub end_user: String,
    pub model: String,
    pub model_group: String,
    pub model_id: String,
    pub custom_llm_provider: String,
    pub api_base: String,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub spend: Decimal,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub start_time: i64,
    pub end_time: i64,
    pub completion_start_time: Option<i64>,
    pub status: String,
    pub error_str: String,
    pub cache_hit: bool,
    pub session_id: String,
    pub trace_id: String,
    pub span_id: String,
    pub request_tags: Vec<String>,
    pub metadata: String,
    pub messages: String,
    pub response: String,
}
