use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::KeySpendRow;

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
