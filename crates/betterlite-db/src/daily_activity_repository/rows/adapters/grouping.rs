use super::super::{DatabaseRow, RowError};
use super::metrics::metrics_row;
use crate::daily_activity_contracts::GroupingSetsRow;

pub fn grouping_rows(rows: &[DatabaseRow]) -> Result<Vec<GroupingSetsRow>, RowError> {
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        decoded.push(GroupingSetsRow {
            metrics: metrics_row(row)?,
            model: row.optional::<String>("model")?,
            model_group: row.optional::<String>("model_group")?,
            custom_llm_provider: row.optional::<String>("custom_llm_provider")?,
            mcp_namespaced_tool_name: row.optional::<String>("mcp_namespaced_tool_name")?,
            endpoint: row.optional::<String>("endpoint")?,
            group_level: row.integer("group_level")?,
            distinct_api_keys: row.optional::<i64>("distinct_api_keys")?,
        });
    }
    Ok(decoded)
}
