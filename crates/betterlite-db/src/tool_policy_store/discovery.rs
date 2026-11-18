use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use tokio_postgres::types::ToSql;

use super::deadline::{StatementBounds, StatementFailure, bounded_statement, discard_uncancelled};
use super::records::{ToolDiscoveryItem, ToolIdGenerator};

const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ToolTable" AS tool
    ("tool_id", "tool_name", "origin", "input_policy", "output_policy", "call_count", "created_by", "updated_by", "key_hash", "team_id", "key_alias", "user_agent", "last_used_at")
VALUES ($1, $2, $3, 'untrusted', 'untrusted', 1, $4, $4, $5, $6, $7, $8, $9)
ON CONFLICT ("tool_name") DO UPDATE SET
    "call_count" = tool."call_count" + 1,
    "updated_at" = $9,
    "last_used_at" = $9"#;

pub async fn batch_upsert_tools(
    client: Option<Client>,
    ids: &dyn ToolIdGenerator,
    bounds: StatementBounds<'_>,
    items: &[ToolDiscoveryItem],
    now: DateTime<Utc>,
) {
    let data = named_items(items);
    if data.is_empty() {
        return;
    }
    let Some(connected) = client else {
        tracing::error!("tool_registry_writer batch_upsert_tools error: no database connection");
        return;
    };
    let count = data.len();
    match upsert_all(&connected, ids, bounds, &data, now).await {
        Ok(()) => tracing::debug!("tool_registry_writer: upserted {count} tool(s)"),
        Err(failure) => {
            tracing::error!("tool_registry_writer batch_upsert_tools error: {failure}");
            discard_uncancelled(connected, &failure);
        }
    }
}

async fn upsert_all(
    client: &Client,
    ids: &dyn ToolIdGenerator,
    bounds: StatementBounds<'_>,
    items: &[(&ToolDiscoveryItem, &str)],
    now: DateTime<Utc>,
) -> Result<(), StatementFailure> {
    let used_at = now.naive_utc();
    for &(item, tool_name) in items {
        let tool_id = ids.next_tool_id();
        let origin = item.origin.as_deref().unwrap_or("user_defined");
        let created_by = item.created_by.as_deref().unwrap_or("system");
        let params: [&(dyn ToSql + Sync); 9] = [
            &tool_id,
            &tool_name,
            &origin,
            &created_by,
            &item.key_hash,
            &item.team_id,
            &item.key_alias,
            &item.user_agent,
            &used_at,
        ];
        bounded_statement(client, bounds, client.execute(UPSERT_SQL, &params)).await?;
    }
    Ok(())
}

fn named_items(items: &[ToolDiscoveryItem]) -> Vec<(&ToolDiscoveryItem, &str)> {
    let mut data = Vec::with_capacity(items.len());
    for item in items {
        if let Some(name) = item.tool_name.as_deref()
            && !name.is_empty()
        {
            data.push((item, name));
        }
    }
    data
}
