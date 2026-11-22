use crate::agent_access::humans::verified_human_agent_grants;
use crate::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use crate::agent_access::resolver::AgentAccessResolver;
use crate::agent_policy::admission::actor_admission_failure;
use crate::agent_policy::{AgentResponse, ManagedAgentContext, ManagedAgentMode};
use crate::agent_registry::AgentRegistry;
use crate::agent_registry::AgentRegistryApi as _;
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::AdmissionError;
use super::ports::AdmissionStore;

mod tests;

pub async fn admit_managed_actor<Store, Grants, Targets, Humans>(
    auth: &mut UserAPIKeyAuth,
    store: Option<&Store>,
    registry: &AgentRegistry,
    access: &AgentAccessResolver<Grants, Targets, Humans>,
) -> Result<(), AdmissionError>
where
    Store: AdmissionStore,
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let delegation_verified = auth.take_delegation_verified();
    let Some(agent_id) = auth.view.token.agent_id.clone() else {
        return Ok(());
    };
    let context = auth.extra.managed_agent_context.clone();
    let Some(store) = store else {
        let registered = registry.get_agent_by_id(&agent_id);
        return without_database(context.as_ref(), registered.as_ref());
    };
    let Some(agent) = stored_agent(store, &agent_id, context.as_ref()).await? else {
        return Ok(());
    };
    admit_stored(auth, agent, context.as_ref(), delegation_verified, access).await
}

// step c: with no database a plain registered agent passes, a managed one cannot be admitted at all
fn without_database(
    context: Option<&ManagedAgentContext>,
    registered: Option<&AgentResponse>,
) -> Result<(), AdmissionError> {
    let managed =
        registered.is_some_and(|agent| agent.identity_managed || agent.identity.is_some());
    if context.is_some() || managed {
        return Err(AdmissionError::from(AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Managed agent policy requires a database"),
        }));
    }
    Ok(())
}

// steps d and e: a failed read goes out, and a missing row is refused only when a context or the history still names it
async fn stored_agent<Store>(
    store: &Store,
    agent_id: &str,
    context: Option<&ManagedAgentContext>,
) -> Result<Option<AgentResponse>, AdmissionError>
where
    Store: AdmissionStore,
{
    let Some(agent) = store.agent(agent_id).await? else {
        let retired = store.retired_agent(agent_id).await?;
        if context.is_some() || retired {
            return Err(AdmissionError::from(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from("Agent no longer exists"),
            }));
        }
        return Ok(None);
    };
    Ok(Some(agent))
}

// steps f to i, then the delegated grant check on the auth that now carries the policy
async fn admit_stored<Grants, Targets, Humans>(
    auth: &mut UserAPIKeyAuth,
    agent: AgentResponse,
    context: Option<&ManagedAgentContext>,
    delegation_verified: bool,
    access: &AgentAccessResolver<Grants, Targets, Humans>,
) -> Result<(), AdmissionError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    if !agent.identity_managed {
        return Ok(());
    }
    let claims = auth
        .extra
        .jwt_claims
        .as_ref()
        .is_some_and(|claims| !claims.0.is_empty());
    if claims && context.is_none() {
        return Err(AdmissionError::from(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("A managed agent requires a matching verified identity"),
        }));
    }
    if let Some(failure) = actor_admission_failure(&agent, context) {
        return Err(AdmissionError::from(failure));
    }
    auth.extra.managed_agent_policy = Some(agent.clone());
    auth.extra.billing_agent_policy = Some(agent.clone());
    auth.extra.requires_fresh_policy = true;
    delegated_grant(
        &agent.agent_id,
        context,
        delegation_verified,
        auth.view.token.team_id.as_deref(),
        access,
    )
    .await
}

// step j: a delegated context the flag did not verify needs the agent in the human's grants
async fn delegated_grant<Grants, Targets, Humans>(
    agent_id: &str,
    context: Option<&ManagedAgentContext>,
    delegation_verified: bool,
    team_id: Option<&str>,
    access: &AgentAccessResolver<Grants, Targets, Humans>,
) -> Result<(), AdmissionError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let Some(context) = context else {
        return Ok(());
    };
    if context.mode != ManagedAgentMode::Delegated || delegation_verified {
        return Ok(());
    }
    let grants = verified_human_agent_grants(access, context.user_id.as_deref(), team_id).await?;
    if grants.contains(agent_id) {
        return Ok(());
    }
    Err(AdmissionError::from(AgentIdentityFailure {
        code: FailureCode::IdentityDenied,
        message: String::from("The delegated user is not permitted to invoke this agent"),
    }))
}
