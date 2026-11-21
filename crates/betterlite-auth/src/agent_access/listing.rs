use std::collections::BTreeSet;

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistryApi as _;
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::user_roles::UserRole;

use super::allowed::is_agent_allowed;
use super::error::AgentAccessError;
use super::reads::{GrantSources, HumanSources, TargetAgents};
use super::resolver::{AgentAccessResolver, resolve_agent_access};

mod tests;

// a listing counts only explicit grants, so Unrestricted access lists nothing
#[must_use]
pub fn granted_ids(access: AgentAccess) -> BTreeSet<String> {
    match access {
        AgentAccess::Unrestricted => BTreeSet::new(),
        AgentAccess::Restricted(agent_ids) => agent_ids,
    }
}

pub async fn granted_agent_ids<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    auth: &UserAPIKeyAuth,
    strict: bool,
) -> Result<BTreeSet<String>, AgentAccessError> {
    let mut granted = BTreeSet::new();
    for context in resolver.humans.effective_contexts(auth).await? {
        granted.extend(granted_ids(
            resolve_agent_access(resolver, Some(&context), strict).await?,
        ));
    }
    Ok(granted)
}

pub async fn accessible_agents<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    auth: &UserAPIKeyAuth,
    all_agents: Option<Vec<AgentResponse>>,
) -> Result<Vec<AgentResponse>, AgentAccessError> {
    let agents = all_agents.unwrap_or_else(|| resolver.registry.get_agent_list(None));
    if auth.extra.user_role == Some(UserRole::ProxyAdmin) {
        return Ok(agents);
    }
    let allowed = granted_agent_ids(resolver, auth, false).await?;
    let mut managed_ids = BTreeSet::new();
    for agent in agents.iter().filter(|agent| agent.identity_managed) {
        if is_agent_allowed(resolver, &agent.agent_id, Some(auth)).await? {
            managed_ids.insert(agent.agent_id.clone());
        }
    }
    Ok(agents
        .into_iter()
        .filter(|agent| {
            if agent.identity_managed {
                managed_ids.contains(&agent.agent_id)
            } else {
                allowed.contains(&agent.agent_id)
            }
        })
        .collect())
}
