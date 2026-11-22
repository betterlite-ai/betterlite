use std::collections::BTreeSet;

use super::error::GrantError;
use crate::auth_model_patterns::access_groups::{
    AccessGroupLookup, ResourceField, collect_resources,
};

mod tests;

// _get_unified_access_group_agents: B747's _get_agent_ids_from_access_groups over the access groups the key or team names
pub(crate) fn agent_ids<Unified>(
    lookup: Option<&Unified>,
    access_group_ids: &[String],
    check_db_only: bool,
) -> Result<BTreeSet<String>, GrantError>
where
    Unified: AccessGroupLookup,
{
    let Some(store) = lookup else {
        return Ok(BTreeSet::new());
    };
    let ids = collect_resources(
        access_group_ids,
        ResourceField::AgentIds,
        Some(store),
        check_db_only,
    )?;
    Ok(ids.into_iter().collect())
}
