use betterlite_db::daily_activity_contracts::DailyActivityRow;
use rust_decimal::Decimal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailySpendRecord {
    pub date: String,
    pub api_key: String,
    pub model: Option<String>,
    pub model_group: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub mcp_namespaced_tool_name: Option<String>,
    pub endpoint: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub spend: Decimal,
    // the reported flat cost: LiteLLM gates the stored value behind PTU attribution before this point
    pub flat_cost: Decimal,
    pub cache_read_input_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub compression_saved_tokens: i64,
    pub compression_savings_spend: Decimal,
    pub prompt_caching_savings_spend: Decimal,
    pub gateway_caching_savings: Decimal,
    pub autorouter_savings_spend: Decimal,
    pub api_requests: i64,
    pub successful_requests: i64,
    pub failed_requests: i64,
    pub total_response_time_ms: i64,
    pub timed_requests: i64,
    // the paginated row declares no entity column, so every row this path converts keys under Unassigned
    pub entity_id: Option<String>,
}

impl From<DailyActivityRow> for DailySpendRecord {
    fn from(row: DailyActivityRow) -> Self {
        Self {
            date: row.date,
            api_key: row.api_key,
            model: row.model,
            model_group: row.model_group,
            custom_llm_provider: row.custom_llm_provider,
            mcp_namespaced_tool_name: row.mcp_namespaced_tool_name,
            endpoint: row.endpoint,
            prompt_tokens: row.prompt_tokens,
            completion_tokens: row.completion_tokens,
            spend: row.spend,
            flat_cost: Decimal::ZERO,
            cache_read_input_tokens: row.cache_read_input_tokens,
            cache_creation_input_tokens: row.cache_creation_input_tokens,
            compression_saved_tokens: row.compression_saved_tokens,
            compression_savings_spend: row.compression_savings_spend,
            prompt_caching_savings_spend: row.prompt_caching_savings_spend,
            gateway_caching_savings: row.gateway_cache_savings_spend,
            autorouter_savings_spend: row.autorouter_savings_spend,
            api_requests: row.api_requests,
            successful_requests: row.successful_requests,
            failed_requests: row.failed_requests,
            total_response_time_ms: row.total_response_time_ms,
            timed_requests: row.timed_requests,
            entity_id: None,
        }
    }
}
