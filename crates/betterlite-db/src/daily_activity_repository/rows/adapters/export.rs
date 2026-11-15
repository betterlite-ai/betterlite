use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::ExportRow;

pub fn export_rows(rows: &[DatabaseRow]) -> Result<Vec<ExportRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(ExportRow {
            date: row.required("date", "a non-null text")?,
            entity_id: row.required("entity_id", "a non-null text")?,
            entity_alias: row.optional::<String>("entity_alias")?,
            api_key: row.optional::<String>("api_key")?,
            key_alias: row.optional::<String>("key_alias")?,
            user_id: row.optional::<String>("user_id")?,
            user_email: row.optional::<String>("user_email")?,
            model: row.optional::<String>("model")?,
            spend: row.required("spend", "a non-null number")?,
            flat_cost: row.required("flat_cost", "a non-null number")?,
            prompt_tokens: row.required("prompt_tokens", "a non-null integer")?,
            completion_tokens: row.required("completion_tokens", "a non-null integer")?,
            api_requests: row.required("api_requests", "a non-null integer")?,
            successful_requests: row.required("successful_requests", "a non-null integer")?,
            failed_requests: row.required("failed_requests", "a non-null integer")?,
            cache_read_input_tokens: row
                .required("cache_read_input_tokens", "a non-null integer")?,
            cache_creation_input_tokens: row
                .required("cache_creation_input_tokens", "a non-null integer")?,
        });
    }
    Ok(decoded)
}
