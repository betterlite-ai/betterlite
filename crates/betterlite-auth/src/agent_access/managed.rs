use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::LiteLLMObjectPermissionTable;
use serde_json::value::RawValue;

use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::admission::managed_agent_policy;
use crate::agent_policy::{AgentResponse, ManagedAgentMode};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::error::AgentAccessError;
use super::humans::verified_human_agent_grants;
use super::listing::granted_ids;
use super::reads::{GrantSources, HumanSources, TargetAgents};
use super::resolver::AgentAccessResolver;

mod tests;

// raw ids throughout: the ceilings, the caller grants and the human grants are never mapped to stable ids
pub async fn managed_actor_agent_access<Grants, Targets, Humans>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    auth: &UserAPIKeyAuth,
) -> Result<AgentAccess, AgentAccessError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let Some(agent) = managed_agent_policy(Some(auth)) else {
        return Ok(AgentAccess::Restricted(BTreeSet::new()));
    };
    let Some(fields) = agent
        .object_permission
        .as_ref()
        .filter(|fields| !fields.is_empty())
    else {
        return Ok(AgentAccess::Restricted(BTreeSet::new()));
    };
    let grouped = grouped_grants(resolver, agent, fields).await?;
    let capped: BTreeSet<String> = match resolver.grants.caller_access(Some(auth), true).await? {
        AgentAccess::Unrestricted => grouped,
        AgentAccess::Restricted(caller_ids) => grouped
            .into_iter()
            .filter(|target| caller_ids.contains(target))
            .collect(),
    };
    let Some(context) = auth.extra.managed_agent_context.as_ref() else {
        return Ok(AgentAccess::Restricted(capped));
    };
    let human_ids = match (&context.mode, context.user_id.as_deref()) {
        (ManagedAgentMode::Autonomous, None | Some(_)) => {
            return Ok(AgentAccess::Restricted(capped));
        }
        (ManagedAgentMode::Delegated, None) => return Ok(AgentAccess::Restricted(BTreeSet::new())),
        (ManagedAgentMode::Delegated, Some(user_id)) => {
            let team_id = auth.view.token.team_id.as_deref();
            verified_human_agent_grants(resolver, Some(user_id), team_id).await?
        }
    };
    Ok(AgentAccess::Restricted(
        capped
            .into_iter()
            .filter(|target| human_ids.contains(target))
            .collect(),
    ))
}

// the agent's own declared agents, kept only where every access group ceiling allows them
async fn grouped_grants<Grants, Targets, Humans>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    agent: &AgentResponse,
    fields: &BTreeMap<String, Box<RawValue>>,
) -> Result<BTreeSet<String>, AgentAccessError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let permission: LiteLLMObjectPermissionTable = serde_json::to_string(fields)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(|source| AgentAccessError::ObjectPermission {
            agent_id: agent.agent_id.clone(),
            source,
        })?;
    let mut own_auth =
        UserAPIKeyAuth::construct(AuthInput::Dict(BTreeMap::new())).map_err(|source| {
            AgentAccessError::Record {
                agent_id: agent.agent_id.clone(),
                source,
            }
        })?;
    own_auth.view.token.object_permission = Some(permission);
    let own = granted_ids(resolver.grants.key_access_strict(&own_auth).await?);
    let access_group_ids = agent.access_group_ids.as_deref().unwrap_or_default();
    let ceilings = resolver
        .grants
        .managed_ceilings(&agent.agent_id, access_group_ids)
        .await?;
    Ok(own
        .into_iter()
        .filter(|target| ceilings.iter().all(|ceiling| ceiling.contains(target)))
        .collect())
}
