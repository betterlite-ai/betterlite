use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use super::execute;
use crate::dashboard_governance::DashboardGovernanceError;
use crate::dashboard_governance::model::ToolIndexRow;

const TOOL_INDEX_INSERT: &str = "INSERT INTO \"LiteLLM_SpendLogToolIndex\" (\"request_id\",\"tool_name\",\"start_time\") VALUES ($1,$2,$3::text::timestamp(3))";

pub(super) async fn insert(
    client: &Client,
    tool_index: &[ToolIndexRow],
) -> Result<(), DashboardGovernanceError> {
    for (row, index) in tool_index.iter().enumerate() {
        let values: &[&(dyn ToSql + Sync)] =
            &[&index.request_id, &index.tool_name, &index.start_time];
        execute(client, "tool_index", row, TOOL_INDEX_INSERT, values).await?;
    }
    Ok(())
}
