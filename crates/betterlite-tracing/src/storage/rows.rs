use betterlite_logging::trace_types::SpanType;
use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanQueryRow {
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub span_type: SpanType,
    pub agent: String,
    pub start_ns: i64,
    pub duration_ns: i64,
    pub status: String,
    pub status_message: Option<String>,
    pub error_truncated: bool,
    pub input_preview: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub litellm_request_id: Option<String>,
    pub team_id: String,
    pub api_key_hash: String,
    pub service: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceListRow {
    pub trace_id: String,
    pub trace_ref: String,
    pub name: String,
    pub service: String,
    pub input_preview: String,
    pub start_ms: i64,
    pub duration_ms: i64,
    pub status: String,
    pub span_count: i64,
    pub agent_count: i64,
    pub agent_invocations: Option<i64>,
    pub llm_calls: i64,
    pub tool_calls: i64,
    pub error_count: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub models: Vec<String>,
    pub request_ids: Vec<String>,
    pub team_id: String,
    pub api_key_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanDetailRow {
    pub span_id: String,
    pub input: String,
    pub output: String,
    pub attributes: IndexMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanErrorRow {
    pub span_id: String,
    pub message: String,
    pub total_chars: usize,
    pub version: String,
}
