use super::super::{DatabaseRow, RowError};
use super::metrics::metrics_row;
use crate::daily_activity_contracts::EntityRollupRow;

pub fn entity_rows(rows: &[DatabaseRow]) -> Result<Vec<EntityRollupRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(EntityRollupRow {
            metrics: metrics_row(row)?,
            entity_id: row.optional::<String>("entity_id")?,
            api_key_rolled: row.integer("api_key_rolled")?,
            distinct_api_keys: row.optional::<i64>("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}
