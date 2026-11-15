use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::RollupMetricsRow;
use rust_decimal::Decimal;

pub fn metrics_row(row: &DatabaseRow) -> Result<RollupMetricsRow, RowError> {
    Ok(RollupMetricsRow {
        date: row.optional::<String>("date")?,
        api_key: row.optional::<String>("api_key")?,
        spend: row.optional::<Decimal>("spend")?,
        ptu_flat_cost: row.optional::<Decimal>("ptu_flat_cost")?,
        prompt_tokens: row.optional::<i64>("prompt_tokens")?,
        completion_tokens: row.optional::<i64>("completion_tokens")?,
        cache_read_input_tokens: row.optional::<i64>("cache_read_input_tokens")?,
        cache_creation_input_tokens: row.optional::<i64>("cache_creation_input_tokens")?,
        compression_saved_tokens: row.optional::<i64>("compression_saved_tokens")?,
        compression_savings_spend: row.optional::<Decimal>("compression_savings_spend")?,
        prompt_caching_savings_spend: row.optional::<Decimal>("prompt_caching_savings_spend")?,
        gateway_cache_savings_spend: row
            .optional::<Decimal>("gateway_injected_caching_savings_spend")?,
        autorouter_savings_spend: row.optional::<Decimal>("autorouter_savings_spend")?,
        api_requests: row.optional::<i64>("api_requests")?,
        successful_requests: row.optional::<i64>("successful_requests")?,
        failed_requests: row.optional::<i64>("failed_requests")?,
        total_response_time_ms: row.optional::<i64>("total_response_time_ms")?,
        timed_requests: row.optional::<i64>("timed_requests")?,
    })
}
