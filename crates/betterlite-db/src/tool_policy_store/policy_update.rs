use chrono::{DateTime, Utc};
use deadpool_postgres::GenericClient;
use uuid::Uuid;

use crate::tool_records::queries::get_tool;
use crate::tool_records::records::{LiteLLMToolTableRow, ToolInputPolicy, ToolOutputPolicy};

use super::records::ToolPolicyUpdate;

const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ToolTable" AS tool
    ("tool_id", "tool_name", "input_policy", "output_policy", "created_by", "updated_by", "created_at", "updated_at")
VALUES ($1, $2, $3, $4, $5, $5, $6, $6)
ON CONFLICT ("tool_name") DO UPDATE SET
    "updated_by" = $5,
    "updated_at" = $6,
    "input_policy" = COALESCE($7::text, tool."input_policy"),
    "output_policy" = COALESCE($8::text, tool."output_policy")"#;

pub async fn update_tool_policy<Client>(
    client: Option<&Client>,
    update: &ToolPolicyUpdate<'_>,
    now: DateTime<Utc>,
) -> Option<LiteLLMToolTableRow>
where
    Client: GenericClient + Sync,
{
    let Some(connected) = client else {
        tracing::error!("tool_registry_writer update_tool_policy error: no database connection");
        return None;
    };
    let updated_by = update.updated_by.unwrap_or("system");
    let created_input = update.input_policy.unwrap_or_default();
    let created_output = update.output_policy.unwrap_or_default();
    let input_update = update.input_policy.map(ToolInputPolicy::as_str);
    let output_update = update.output_policy.map(output_policy_text);
    let tool_id = Uuid::new_v4().to_string();
    let touched_at = now.naive_utc();
    let written = connected
        .execute(
            UPSERT_SQL,
            &[
                &tool_id,
                &update.tool_name,
                &created_input.as_str(),
                &output_policy_text(created_output),
                &updated_by,
                &touched_at,
                &input_update,
                &output_update,
            ],
        )
        .await;
    if let Err(error) = written {
        tracing::error!("tool_registry_writer update_tool_policy error: {error}");
        return None;
    }
    get_tool(Some(connected), update.tool_name).await
}

const fn output_policy_text(policy: ToolOutputPolicy) -> &'static str {
    match policy {
        ToolOutputPolicy::Trusted => "trusted",
        ToolOutputPolicy::Untrusted => "untrusted",
    }
}
