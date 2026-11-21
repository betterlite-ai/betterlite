use std::collections::BTreeSet;

use super::AgentGrants;
use super::error::GrantError;
use crate::agent_access_groups::access::to_stable_ids;
use crate::agent_access_groups::reads::AccessGroupReads;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// _agent_access_group_ceiling: the agent's own access group ceiling, mapped to stable ids; a read error passes through
pub(crate) async fn agent_access_group_ceiling<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<Option<BTreeSet<String>>, GrantError>
where
    Reads: AccessGroupReads,
{
    let Some(auth) = auth else {
        return Ok(None);
    };
    let Some(agent_id) = auth.view.token.agent_id.as_deref() else {
        return Ok(None);
    };
    let ceiling = grants
        .ceilings
        .resolve_agent_ceiling(agent_id, strict)
        .await?;
    Ok(ceiling.map(|found| to_stable_ids(&grants.registry, &found.agent_ids)))
}
