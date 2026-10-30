use betterlite_logging::trace_types::TraceScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListTracesQuery {
    pub scope: TraceScope,
    pub start_ms: i64,
    pub end_ms: i64,
    pub cursor_ms: i64,
    pub cursor_trace_id: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSpansQuery {
    pub scope: TraceScope,
    pub trace_id: String,
    pub trace_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanDetailQuery {
    pub scope: TraceScope,
    pub trace_id: String,
    pub span_id: String,
    pub trace_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanErrorQuery {
    pub scope: TraceScope,
    pub trace_id: String,
    pub span_id: String,
    pub trace_ref: String,
    pub error_offset: u64,
    pub error_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendByRequestIdsQuery {
    pub scope: TraceScope,
    pub request_ids: Vec<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}
