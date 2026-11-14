use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::{CREATED_AT, execute, measured_key_hash};
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::ToolRow;
use crate::seed::FixtureSeed;

const TOOL_INSERT: &str = "INSERT INTO \"LiteLLM_ToolTable\" (\"tool_id\",\"tool_name\",\"origin\",\"input_policy\",\"output_policy\",\"call_count\",\"key_hash\",\"team_id\",\"key_alias\",\"user_agent\",\"last_used_at\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::text::timestamp(3),$12::text::timestamp(3),$13::text::timestamp(3))";

pub(super) async fn insert(
    client: &Client,
    tools: &[ToolRow],
    fixture: &FixtureSeed,
) -> Result<(), DashboardGovernanceError> {
    for (row, tool) in tools.iter().enumerate() {
        let key_hash = match tool.key_index {
            Some(index) => Some(measured_key_hash("tools", row, index, fixture)?),
            None => None,
        };
        let input_policy = tool.input_policy.as_wire();
        let output_policy = tool.output_policy.as_wire();
        let values: &[&(dyn ToSql + Sync)] = &[
            &tool.tool_id,
            &tool.tool_name,
            &tool.origin,
            &input_policy,
            &output_policy,
            &tool.call_count,
            &key_hash,
            &tool.team_id,
            &tool.key_alias,
            &tool.user_agent,
            &tool.last_used_at,
            &CREATED_AT,
            &CREATED_AT,
        ];
        execute(client, "tools", row, TOOL_INSERT, values).await?;
    }
    Ok(())
}
