use crate::daily_activity_contracts::identifiers::DailyActivityTable;

#[must_use]
pub fn ptu_flat_cost_select(table: DailyActivityTable, aggregate: bool) -> String {
    match (table, aggregate) {
        (DailyActivityTable::Team, true) => "SUM(ptu_flat_cost)::float AS ptu_flat_cost".to_owned(),
        (DailyActivityTable::Team, false) => "SUM(scoped.ptu_flat_cost)::float".to_owned(),
        (
            DailyActivityTable::User
            | DailyActivityTable::Tag
            | DailyActivityTable::Organization
            | DailyActivityTable::Customer
            | DailyActivityTable::Agent,
            true,
        ) => "0::float AS ptu_flat_cost".to_owned(),
        (
            DailyActivityTable::User
            | DailyActivityTable::Tag
            | DailyActivityTable::Organization
            | DailyActivityTable::Customer
            | DailyActivityTable::Agent,
            false,
        ) => "0::float".to_owned(),
    }
}

#[must_use]
pub fn rollup_metric_select(table: DailyActivityTable) -> String {
    let ptu = ptu_flat_cost_select(table, true);
    format!(
        "SUM(spend)::float AS spend,\n            {ptu},\n            SUM(prompt_tokens)::bigint AS prompt_tokens,\n            SUM(completion_tokens)::bigint AS completion_tokens,\n            SUM(cache_read_input_tokens)::bigint AS cache_read_input_tokens,\n            SUM(cache_creation_input_tokens)::bigint AS cache_creation_input_tokens,\n            SUM(compression_saved_tokens)::bigint AS compression_saved_tokens,\n            SUM(compression_savings_spend)::float AS compression_savings_spend,\n            SUM(prompt_caching_savings_spend)::float AS prompt_caching_savings_spend,\n            SUM(gateway_injected_caching_savings_spend)::float AS gateway_injected_caching_savings_spend,\n            SUM(autorouter_savings_spend)::float AS autorouter_savings_spend,\n            SUM(api_requests)::bigint AS api_requests,\n            SUM(successful_requests)::bigint AS successful_requests,\n            SUM(failed_requests)::bigint AS failed_requests,\n            SUM(total_response_time_ms)::bigint AS total_response_time_ms,\n            SUM(timed_requests)::bigint AS timed_requests"
    )
}

pub const KEY_SPEND_SELECT: &str = "COALESCE(SUM(spend), 0)::float AS spend,\n            COALESCE(SUM(prompt_tokens), 0)::bigint AS prompt_tokens,\n            COALESCE(SUM(completion_tokens), 0)::bigint AS completion_tokens,\n            (COALESCE(SUM(prompt_tokens), 0) + COALESCE(SUM(completion_tokens), 0))::bigint AS total_tokens,\n            COALESCE(SUM(api_requests), 0)::bigint AS api_requests,\n            COALESCE(SUM(successful_requests), 0)::bigint AS successful_requests,\n            COALESCE(SUM(failed_requests), 0)::bigint AS failed_requests,\n            COALESCE(SUM(cache_read_input_tokens), 0)::bigint AS cache_read_input_tokens,\n            COALESCE(SUM(cache_creation_input_tokens), 0)::bigint AS cache_creation_input_tokens";
