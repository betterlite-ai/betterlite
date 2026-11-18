use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use uuid::Uuid;

use super::records::ToolDiscoveryItem;

const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ToolTable" AS tool
    ("tool_id", "tool_name", "origin", "input_policy", "output_policy", "call_count", "created_by", "updated_by", "key_hash", "team_id", "key_alias", "user_agent", "last_used_at")
VALUES ($1, $2, $3, 'untrusted', 'untrusted', 1, $4, $4, $5, $6, $7, $8, $9)
ON CONFLICT ("tool_name") DO UPDATE SET
    "call_count" = tool."call_count" + 1,
    "updated_at" = $9,
    "last_used_at" = $9"#;

pub async fn batch_upsert_tools<Client>(
    client: Option<&Client>,
    items: &[ToolDiscoveryItem],
    now: DateTime<Utc>,
) where
    Client: GenericClient + Sync,
{
    let mut data: Vec<(&ToolDiscoveryItem, &str)> = Vec::with_capacity(items.len());
    for item in items {
        if let Some(name) = item.tool_name.as_deref()
            && !name.is_empty()
        {
            data.push((item, name));
        }
    }
    if data.is_empty() {
        return;
    }
    let Some(connected) = client else {
        tracing::error!("tool_registry_writer batch_upsert_tools error: no database connection");
        return;
    };
    let used_at = now.naive_utc();
    let count = data.len();
    for (item, tool_name) in data {
        let tool_id = Uuid::new_v4().to_string();
        let origin = item.origin.as_deref().unwrap_or("user_defined");
        let created_by = item.created_by.as_deref().unwrap_or("system");
        let written = connected
            .execute(
                UPSERT_SQL,
                &[
                    &tool_id,
                    &tool_name,
                    &origin,
                    &created_by,
                    &item.key_hash,
                    &item.team_id,
                    &item.key_alias,
                    &item.user_agent,
                    &used_at,
                ],
            )
            .await;
        if let Err(error) = written {
            tracing::error!("tool_registry_writer batch_upsert_tools error: {error}");
            return;
        }
    }
    tracing::debug!("tool_registry_writer: upserted {count} tool(s)");
}
