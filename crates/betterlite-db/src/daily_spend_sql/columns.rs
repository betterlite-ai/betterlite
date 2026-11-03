pub(super) const KEY_COLUMNS: [&str; 6] = [
    "date",
    "api_key",
    "model",
    "custom_llm_provider",
    "mcp_namespaced_tool_name",
    "endpoint",
];

pub(super) const COUNTER_COLUMNS: [&str; 10] = [
    "prompt_tokens",
    "completion_tokens",
    "api_requests",
    "successful_requests",
    "failed_requests",
    "cache_read_input_tokens",
    "cache_creation_input_tokens",
    "compression_saved_tokens",
    "total_response_time_ms",
    "timed_requests",
];

pub(super) const SPEND_COLUMNS: [&str; 5] = [
    "spend",
    "compression_savings_spend",
    "prompt_caching_savings_spend",
    "gateway_injected_caching_savings_spend",
    "autorouter_savings_spend",
];

// The unique conflict tuple is NULL-free because NULL never matches itself in a unique index.
pub(super) fn sql_cast(column: &str) -> &'static str {
    if COUNTER_COLUMNS.contains(&column) {
        "bigint"
    } else if SPEND_COLUMNS.contains(&column) {
        "double precision"
    } else {
        "text"
    }
}
