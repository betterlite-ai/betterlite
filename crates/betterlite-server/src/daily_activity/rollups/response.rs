use betterlite_analytics::daily_metrics::{DailySpendMetadata, SpendMetrics};
use betterlite_db::daily_activity_contracts::EntityRollupRow;
use indexmap::IndexMap;

use super::metadata::entity_name;

pub(super) fn entity_key_totals(
    entity_rows: Option<&[EntityRollupRow]>,
    include_entity_breakdown: bool,
) -> Option<IndexMap<String, i64>> {
    if !include_entity_breakdown {
        return None;
    }
    let mut totals = IndexMap::new();
    for row in entity_rows.unwrap_or_default() {
        let Some(count) = row.distinct_api_keys else {
            continue;
        };
        if row.api_key_rolled == 0 {
            continue;
        }
        totals.insert(entity_name(row.entity_id.as_deref()).to_owned(), count);
    }
    Some(totals)
}

pub(super) const fn response_metadata(
    totals: &SpendMetrics,
    api_key_limit: i64,
    total_api_keys: i64,
    entity_total_api_keys: Option<IndexMap<String, i64>>,
) -> DailySpendMetadata {
    DailySpendMetadata {
        total_spend: totals.spend,
        total_flat_cost: totals.flat_cost,
        total_prompt_tokens: totals.prompt_tokens,
        total_completion_tokens: totals.completion_tokens,
        total_tokens: totals.total_tokens,
        total_api_requests: totals.api_requests,
        total_successful_requests: totals.successful_requests,
        total_failed_requests: totals.failed_requests,
        total_cache_read_input_tokens: totals.cache_read_input_tokens,
        total_cache_creation_tokens: totals.cache_creation_input_tokens,
        total_compression_saved_tokens: totals.compression_saved_tokens,
        total_compression_savings: totals.compression_savings_spend,
        total_prompt_caching_savings: totals.prompt_caching_savings_spend,
        total_gateway_caching_savings: totals.gateway_caching_savings,
        total_autorouter_savings_spend: totals.autorouter_savings_spend,
        total_response_time_ms: totals.total_response_time_ms,
        total_timed_requests: totals.timed_requests,
        page: 1,
        total_pages: 1,
        has_more: false,
        api_key_limit: Some(api_key_limit),
        total_api_keys: Some(total_api_keys),
        entity_total_api_keys,
    }
}
