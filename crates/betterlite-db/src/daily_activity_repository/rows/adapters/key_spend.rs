use super::super::{DatabaseRow, RowError};
use crate::daily_activity_contracts::KeySpendRow;

pub fn key_spend_rows(rows: &[DatabaseRow]) -> Result<Vec<KeySpendRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(KeySpendRow {
            api_key: row.required("api_key", "a non-null text")?,
            spend: row.required("spend", "a non-null number")?,
            prompt_tokens: row.required("prompt_tokens", "a non-null integer")?,
            completion_tokens: row.required("completion_tokens", "a non-null integer")?,
            total_tokens: row.required("total_tokens", "a non-null integer")?,
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
