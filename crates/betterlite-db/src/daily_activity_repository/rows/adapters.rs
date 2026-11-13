use super::{DatabaseRow, DatabaseValue, RowError};
use crate::daily_activity_contracts::{
    EntityRollupRow, ExportRow, GroupingSetsRow, KeyPage, KeySpendRow, RollupMetricsRow,
};

fn metrics_row(row: &DatabaseRow) -> Result<RollupMetricsRow, RowError> {
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

pub fn grouping_rows(rows: &[DatabaseRow]) -> Result<Vec<GroupingSetsRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(GroupingSetsRow {
            metrics: metrics_row(row)?,
            model: row.optional_text("model")?,
            model_group: row.optional_text("model_group")?,
            custom_llm_provider: row.optional_text("custom_llm_provider")?,
            mcp_namespaced_tool_name: row.optional_text("mcp_namespaced_tool_name")?,
            endpoint: row.optional_text("endpoint")?,
            group_level: row.integer("group_level")?,
            distinct_api_keys: row.optional_integer("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}

pub fn entity_rows(rows: &[DatabaseRow]) -> Result<Vec<EntityRollupRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(EntityRollupRow {
            metrics: metrics_row(row)?,
            entity_id: row.optional_text("entity_id")?,
            api_key_rolled: row.integer("api_key_rolled")?,
            distinct_api_keys: row.optional_integer("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}

pub fn key_spend_rows(rows: &[DatabaseRow]) -> Result<Vec<KeySpendRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(KeySpendRow {
            api_key: row.text("api_key")?,
            spend: row.decimal("spend")?,
            prompt_tokens: row.integer("prompt_tokens")?,
            completion_tokens: row.integer("completion_tokens")?,
            total_tokens: row.integer("total_tokens")?,
            api_requests: row.integer("api_requests")?,
            successful_requests: row.integer("successful_requests")?,
            failed_requests: row.integer("failed_requests")?,
            cache_read_input_tokens: row.integer("cache_read_input_tokens")?,
            cache_creation_input_tokens: row.integer("cache_creation_input_tokens")?,
        });
    }
    Ok(decoded)
}

pub fn key_page(rows: &[DatabaseRow]) -> Result<KeyPage, RowError> {
    let total_api_keys = match rows.first() {
        None => 0,
        Some(row) => row.integer_if_present("total_api_keys")?.unwrap_or(0),
    };
    let retained: Vec<DatabaseRow> = rows
        .iter()
        .filter(|row| !matches!(row.values.get("api_key"), None | Some(DatabaseValue::Null)))
        .cloned()
        .collect();
    Ok(KeyPage {
        rows: key_spend_rows(&retained)?,
        total_api_keys,
    })
}

pub fn export_rows(rows: &[DatabaseRow]) -> Result<Vec<ExportRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(ExportRow {
            date: row.text("date")?,
            entity_id: row.text("entity_id")?,
            entity_alias: row.optional_text("entity_alias")?,
            api_key: row.optional_text("api_key")?,
            key_alias: row.optional_text("key_alias")?,
            user_id: row.optional_text("user_id")?,
            user_email: row.optional_text("user_email")?,
            model: row.optional_text("model")?,
            spend: row.decimal("spend")?,
            flat_cost: row.decimal("flat_cost")?,
            prompt_tokens: row.integer("prompt_tokens")?,
            completion_tokens: row.integer("completion_tokens")?,
            api_requests: row.integer("api_requests")?,
            successful_requests: row.integer("successful_requests")?,
            failed_requests: row.integer("failed_requests")?,
            cache_read_input_tokens: row.integer("cache_read_input_tokens")?,
            cache_creation_input_tokens: row.integer("cache_creation_input_tokens")?,
        });
    }
    Ok(decoded)
}
