use std::collections::BTreeSet;

use super::AgentGrants;
// ast-grep-ignore: no-long-names
use super::config_agent_ids_for_access_groups::config_agent_ids;
use super::db_agent_ids_for_access_groups::db_agent_ids;
use super::error::GrantError;
use super::reads::AgentTableReads;
use crate::agent_registry::AgentRegistryApi as _;

// _get_agents_from_access_groups: the config records first, then the agents table, unioned into one set
pub(crate) async fn agent_ids<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    groups: &[String],
    check_db_only: bool,
) -> Result<BTreeSet<String>, GrantError>
where
    Table: AgentTableReads,
{
    let configured = config_agent_ids(&grants.registry.get_agent_list(None), groups);
    let stored = db_agent_ids(&grants.agents, groups, check_db_only).await?;
    Ok(configured.union(&stored).cloned().collect())
}
