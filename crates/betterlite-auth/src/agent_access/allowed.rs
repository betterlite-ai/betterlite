use crate::agent_access_groups::access::AgentAccess;
use crate::agent_policy::AgentResponse;
use crate::agent_policy::admission::managed_agent_policy;
use crate::agent_registry::AgentRegistryApi as _;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::AgentAccessError;
use super::listing::granted_agent_ids;
use super::reads::{GrantSources, HumanSources, TargetAgents};
use super::resolver::{AgentAccessResolver, resolve_agent_access};

#[path = "allowed/managed_target/tests.rs"]
mod managed_target;
#[path = "allowed/registry_ids/tests.rs"]
mod registry_ids;

pub async fn is_agent_allowed<Grants: GrantSources, Targets: TargetAgents, Humans: HumanSources>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    agent_id: &str,
    auth: Option<&UserAPIKeyAuth>,
) -> Result<bool, AgentAccessError> {
    let registry_managed = resolver
        .registry
        .get_agent_by_id(agent_id)
        .is_some_and(|registered| registered.identity_managed);
    if registry_managed || resolver.targets.has_database() {
        let target = resolver
            .targets
            .agent(agent_id)
            .await
            .map_err(AgentAccessError::Identity)?;
        if target.is_none() && registry_managed {
            return Ok(false);
        }
        // a row that is not identity managed falls through to the ordinary grant check
        if let Some(managed) = target.filter(|row| row.identity_managed) {
            return managed_target_allowed(resolver, &managed, auth).await;
        }
    }
    let AgentAccess::Restricted(allowed_ids) = resolve_agent_access(resolver, auth, false).await?
    else {
        return Ok(true);
    };
    let stable_id = resolver.registry.stable_agent_id(agent_id);
    Ok(!resolver
        .registry
        .ids_for_agent(&stable_id)
        .is_disjoint(&allowed_ids))
}

// a managed target answers only from explicit, freshly loaded grants
async fn managed_target_allowed<
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
>(
    resolver: &AgentAccessResolver<Grants, Targets, Humans>,
    target: &AgentResponse,
    auth: Option<&UserAPIKeyAuth>,
) -> Result<bool, AgentAccessError> {
    let identity_active = target
        .identity
        .as_ref()
        .is_some_and(|identity| identity.active.0);
    let Some(caller) = auth.filter(|_| target.enabled.0 && identity_active) else {
        return Ok(false);
    };
    let key_hash = caller
        .extra
        .api_key
        .0
        .as_ref()
        .or(caller.view.token.token.as_ref());
    // a managed key, a session token and a custom-auth key carry their authority already
    let plain_key = managed_agent_policy(auth).is_none()
        && !caller.extra.is_session_token
        && !caller.extra.provenance.authenticated_by_custom_auth;
    let mut fresh_auth = match key_hash.filter(|_| plain_key) {
        Some(hash) => resolver.humans.reload_admitted_key(hash, true).await?,
        None => caller.clone(),
    };
    fresh_auth.extra.requires_fresh_policy = true;
    // the forwarded caller cap comes from the request, never from the reloaded key
    fresh_auth
        .extra
        .agent_caller
        .clone_from(&caller.extra.agent_caller);
    let explicit = granted_agent_ids(resolver, &fresh_auth, true).await?;
    Ok(explicit.contains(&target.agent_id))
}
