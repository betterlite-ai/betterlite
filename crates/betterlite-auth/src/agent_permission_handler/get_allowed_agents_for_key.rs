use std::collections::BTreeSet;

use tracing::warn;

use super::AgentGrants;
use super::db_agent_ids_for_access_groups::db_agent_ids;
use super::error::GrantError;
use super::reads::AgentTableReads;
use super::unified_access_group;
use crate::agent_access_groups::access::AgentAccess;
use crate::auth_model_patterns::access_groups::AccessGroupLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

mod tests;

// get_allowed_agents_for_key: a key that declares nothing reaches every agent, otherwise the three sources union
pub(crate) async fn allowed_agents_for_key<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let Some(auth) = auth else {
        return Ok(AgentAccess::Unrestricted);
    };
    match declared_key_access(grants, auth, strict).await {
        Ok(access) => Ok(access),
        Err(error) if strict => Err(GrantError::PolicyUnavailable(Box::new(error))),
        Err(error) => {
            warn!("Failed to get allowed agents for key: {error}");
            Ok(AgentAccess::Unrestricted)
        }
    }
}

async fn declared_key_access<Reads, Table, Unified>(
    grants: &AgentGrants<Reads, Table, Unified>,
    auth: &UserAPIKeyAuth,
    strict: bool,
) -> Result<AgentAccess, GrantError>
where
    Table: AgentTableReads,
    Unified: AccessGroupLookup,
{
    let token = &auth.view.token;
    let permission = token.object_permission.as_ref();
    let direct = permission
        .and_then(|declared| declared.agents.0.as_deref())
        .unwrap_or(&[]);
    let declared = permission
        .and_then(|declared| declared.agent_access_groups.0.as_deref())
        .unwrap_or(&[]);
    let unified = token.access_group_ids.as_deref().unwrap_or(&[]);
    if direct.is_empty() && declared.is_empty() && unified.is_empty() {
        return Ok(AgentAccess::Unrestricted);
    }
    let mut ids: BTreeSet<String> = direct.iter().cloned().collect();
    if !declared.is_empty() {
        ids.extend(db_agent_ids(&grants.agents, declared, strict).await?);
    }
    if !unified.is_empty() {
        ids.extend(unified_access_group::agent_ids(
            grants.unified.as_ref(),
            unified,
            strict,
        )?);
    }
    Ok(AgentAccess::Restricted(ids))
}
