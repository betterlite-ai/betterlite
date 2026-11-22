use std::collections::BTreeSet;

use super::error::GrantError;
use super::reads::AgentTableReads;

mod tests;

// _get_db_agent_ids_for_access_groups: the hasSome filter runs in the port, and no table or no name reads nothing
pub(crate) async fn db_agent_ids<Table>(
    table: &Table,
    groups: &[String],
    check_db_only: bool,
) -> Result<BTreeSet<String>, GrantError>
where
    Table: AgentTableReads,
{
    if !table.has_database() || groups.is_empty() {
        return Ok(BTreeSet::new());
    }
    let rows = table
        .agent_ids_with_access_groups(groups, check_db_only)
        .await?;
    Ok(rows.into_iter().collect())
}
