use chrono::{DateTime, Utc};
use deadpool_postgres::Client;
use tokio::time::timeout;
use tokio_postgres::types::ToSql;

use crate::tool_records::queries::get_tool;
use crate::tool_records::records::{LiteLLMToolTableRow, ToolInputPolicy, ToolOutputPolicy};

use super::deadline::{StatementBounds, StatementFailure, bounded_statement, cancel_statement};
use super::records::{ToolIdGenerator, ToolPolicyUpdate};

const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_ToolTable" AS tool
    ("tool_id", "tool_name", "input_policy", "output_policy", "created_by", "updated_by", "created_at", "updated_at")
VALUES ($1, $2, $3, $4, $5, $5, $6, $6)
ON CONFLICT ("tool_name") DO UPDATE SET
    "updated_by" = $5,
    "updated_at" = $6,
    "input_policy" = COALESCE($7::text, tool."input_policy"),
    "output_policy" = COALESCE($8::text, tool."output_policy")"#;

pub async fn update_tool_policy(
    client: Option<Client>,
    ids: &dyn ToolIdGenerator,
    bounds: StatementBounds<'_>,
    update: &ToolPolicyUpdate<'_>,
    now: DateTime<Utc>,
) -> Option<LiteLLMToolTableRow> {
    let Some(connected) = client else {
        tracing::error!("tool_registry_writer update_tool_policy error: no database connection");
        return None;
    };
    match write_policy(&connected, ids, bounds, update, now).await {
        Ok(()) => (),
        Err(failure) => {
            tracing::error!("tool_registry_writer update_tool_policy error: {failure}");
            if let StatementFailure::Expired {
                cancelled: false, ..
            } = failure
            {
                // taking the client out of the pool and dropping it aborts the connection task
                let detached = deadpool_postgres::Object::take(connected);
                drop(detached);
            }
            return None;
        }
    }
    match read_back(&connected, bounds, update.tool_name).await {
        Ok(row) => row,
        Err(failure) => {
            tracing::error!("tool_registry_writer update_tool_policy error: {failure}");
            if let StatementFailure::Expired {
                cancelled: false, ..
            } = failure
            {
                // taking the client out of the pool and dropping it aborts the connection task
                let detached = deadpool_postgres::Object::take(connected);
                drop(detached);
            }
            None
        }
    }
}

async fn write_policy(
    client: &Client,
    ids: &dyn ToolIdGenerator,
    bounds: StatementBounds<'_>,
    update: &ToolPolicyUpdate<'_>,
    now: DateTime<Utc>,
) -> Result<(), StatementFailure> {
    let updated_by = update.updated_by.unwrap_or("system");
    let created_input = update.input_policy.unwrap_or_default();
    let created_output = update.output_policy.unwrap_or_default();
    let input_update = update.input_policy.map(ToolInputPolicy::as_str);
    let output_update = update.output_policy.map(output_policy_text);
    let tool_id = ids.next_tool_id();
    let touched_at = now.naive_utc();
    let params: [&(dyn ToSql + Sync); 8] = [
        &tool_id,
        &update.tool_name,
        &created_input.as_str(),
        &output_policy_text(created_output),
        &updated_by,
        &touched_at,
        &input_update,
        &output_update,
    ];
    bounded_statement(client, bounds, client.execute(UPSERT_SQL, &params)).await?;
    Ok(())
}

async fn read_back(
    client: &Client,
    bounds: StatementBounds<'_>,
    tool_name: &str,
) -> Result<Option<LiteLLMToolTableRow>, StatementFailure> {
    match timeout(bounds.deadline, get_tool(Some(client), tool_name)).await {
        Ok(row) => Ok(row),
        Err(_deadline_elapsed) => Err(StatementFailure::Expired {
            deadline: bounds.deadline,
            cancelled: cancel_statement(client, bounds).await,
        }),
    }
}

const fn output_policy_text(policy: ToolOutputPolicy) -> &'static str {
    match policy {
        ToolOutputPolicy::Trusted => "trusted",
        ToolOutputPolicy::Untrusted => "untrusted",
    }
}
