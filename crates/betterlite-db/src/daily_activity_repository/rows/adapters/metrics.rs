use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::RollupMetricsRow;

pub fn metrics_row(row: &DatabaseRow) -> Result<RollupMetricsRow, RowError> {
    Ok(RollupMetricsRow {
        date: row.optional_text("date")?,
        api_key: row.optional_text("api_key")?,
        spend: row.optional_decimal("spend")?,
        ptu_flat_cost: row.optional_decimal("ptu_flat_cost")?,
        prompt_tokens: row.optional_integer("prompt_tokens")?,
        completion_tokens: row.optional_integer("completion_tokens")?,
        cache_read_input_tokens: row.optional_integer("cache_read_input_tokens")?,
        cache_creation_input_tokens: row.optional_integer("cache_creation_input_tokens")?,
        compression_saved_tokens: row.optional_integer("compression_saved_tokens")?,
        compression_savings_spend: row.optional_decimal("compression_savings_spend")?,
        prompt_caching_savings_spend: row.optional_decimal("prompt_caching_savings_spend")?,
        gateway_cache_savings_spend: row
            .optional_decimal("gateway_injected_caching_savings_spend")?,
        autorouter_savings_spend: row.optional_decimal("autorouter_savings_spend")?,
        api_requests: row.optional_integer("api_requests")?,
        successful_requests: row.optional_integer("successful_requests")?,
        failed_requests: row.optional_integer("failed_requests")?,
        total_response_time_ms: row.optional_integer("total_response_time_ms")?,
        timed_requests: row.optional_integer("timed_requests")?,
    })
}
