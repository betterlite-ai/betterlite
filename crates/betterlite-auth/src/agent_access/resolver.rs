use std::collections::BTreeSet;
use std::sync::Arc;

use crate::agent_access_groups::access::{AgentAccess, intersect_agent_access};
use crate::agent_policy::admission::managed_agent_policy;
use crate::agent_registry::AgentRegistry;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::AgentAccessError;
use super::managed::managed_actor_agent_access;
use super::reads::{GrantSources, HumanSources, TargetAgents};

#[path = "resolver/strict_mode/tests.rs"]
mod strict_mode;
mod tests;

// The workspace denies multiple_inherent_impl, so each check is a free function in its own module
pub struct AgentAccessResolver<Grants, Targets, Humans> {
    pub registry: Arc<AgentRegistry>,
    pub grants: Grants,
    pub targets: Targets,
    pub humans: Humans,
}

pub async fn resolve_agent_access<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    auth: Option<&UserAPIKeyAuth>,
    strict: bool,
) -> Result<AgentAccess, AgentAccessError> {
    if let Some(managed_auth) = auth
        && managed_agent_policy(auth).is_some()
    {
        return managed_actor_agent_access(resolver, managed_auth).await;
    }
    let key_team = resolver.grants.key_team_access(auth, strict).await?;
    // a strict caller with no grant gets nothing, before a ceiling could turn everything into a grant
    if strict && matches!(key_team, AgentAccess::Unrestricted) {
        return Ok(AgentAccess::Restricted(BTreeSet::new()));
    }
    let caller = resolver.grants.caller_access(auth, strict).await?;
    let own = intersect_agent_access(&resolver.registry, &key_team, &caller);
    let Some(ceiling) = resolver.grants.access_group_ceiling(auth, strict).await? else {
        return Ok(own);
    };
    Ok(match own {
        AgentAccess::Unrestricted => AgentAccess::Restricted(ceiling),
        AgentAccess::Restricted(own_ids) => AgentAccess::Restricted(
            own_ids
                .into_iter()
                .filter(|agent_id| ceiling.contains(agent_id))
                .collect(),
        ),
    })
}
