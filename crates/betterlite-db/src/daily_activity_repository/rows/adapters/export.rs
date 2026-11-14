use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::ExportRow;

pub fn export_rows(rows: &[DatabaseRow]) -> Result<Vec<ExportRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(ExportRow {
            date: row.text("date")?,
            entity_id: row.text("entity_id")?,
            entity_alias: row.optional::<String>("entity_alias")?,
            api_key: row.optional::<String>("api_key")?,
            key_alias: row.optional::<String>("key_alias")?,
            user_id: row.optional::<String>("user_id")?,
            user_email: row.optional::<String>("user_email")?,
            model: row.optional::<String>("model")?,
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
