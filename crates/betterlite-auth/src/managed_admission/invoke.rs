use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::agent_access::allowed::is_agent_allowed;
use crate::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use crate::agent_access::resolver::AgentAccessResolver;
use crate::agent_policy::AgentResponse;
use crate::agent_registry::AgentRegistryApi as _;
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::model_budget_values::amount_text::amount_from_text;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::AdmissionError;
use super::ports::{AdmissionStore, InvocationSources, ReadThrough};

mod tests;

pub async fn prepare_agent_invocation<Store, Read, Grants, Targets, Humans>(
    auth: &mut UserAPIKeyAuth,
    target_name: &str,
    billable: bool,
    sources: &InvocationSources<'_, Store, Read>,
    access: &AgentAccessResolver<Grants, Targets, Humans>,
) -> Result<(), AdmissionError>
where
    Store: AdmissionStore,
    Read: ReadThrough,
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    let Some(registered) = lookup(access, sources.read_through, target_name).await else {
        return Ok(());
    };
    let managed = registered.identity_managed || registered.identity.is_some();
    let Some(store) = sources.store else {
        if managed {
            return Err(AdmissionError::from(AgentIdentityFailure {
                code: FailureCode::PolicyUnavailable,
                message: String::from("Managed agent policy requires a database"),
            }));
        }
        return prepare_target(auth, registered, billable, access).await;
    };
    let Some(target) = store.agent(&registered.agent_id).await? else {
        if managed {
            return Err(AdmissionError::from(AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from("Invoked agent no longer exists"),
            }));
        }
        return prepare_target(auth, registered, billable, access).await;
    };
    prepare_target(auth, target, billable, access).await
}

// get_agent_with_read_through: by id, then by name, then one read-through attempt and the same lookups once
async fn lookup<Grants, Targets, Humans, Read>(
    access: &AgentAccessResolver<Grants, Targets, Humans>,
    read_through: &Read,
    target_name: &str,
) -> Option<AgentResponse>
where
    Read: ReadThrough,
{
    if let Some(found) = registry_agent(access, target_name) {
        return Some(found);
    }
    if !read_through.attempt(target_name).await {
        return None;
    }
    registry_agent(access, target_name)
}

fn registry_agent<Grants, Targets, Humans>(
    access: &AgentAccessResolver<Grants, Targets, Humans>,
    target_name: &str,
) -> Option<AgentResponse> {
    access
        .registry
        .get_agent_by_id(target_name)
        .or_else(|| access.registry.get_agent_by_name(target_name))
}

// steps g to j: the caller check, the invocation marks and the price of the effective agent
async fn prepare_target<Grants, Targets, Humans>(
    auth: &mut UserAPIKeyAuth,
    effective: AgentResponse,
    billable: bool,
    access: &AgentAccessResolver<Grants, Targets, Humans>,
) -> Result<(), AdmissionError>
where
    Grants: GrantSources,
    Targets: TargetAgents,
    Humans: HumanSources,
{
    if !effective.identity_managed && auth.extra.managed_agent_policy.is_none() {
        return Ok(());
    }
    if !is_agent_allowed(access, &effective.agent_id, Some(&*auth)).await? {
        return Err(AdmissionError::from(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("The caller is not permitted to invoke this agent"),
        }));
    }
    auth.extra.invoked_agent_id = Some(effective.agent_id.clone());
    auth.extra.invoked_agent_policy = Some(effective.clone());
    if auth.view.token.agent_id.is_none() && effective.identity_managed {
        auth.extra.billing_agent_policy = Some(effective.clone());
    }
    let Some(price) = invocation_price(&effective, billable) else {
        return Err(AdmissionError::from(AgentIdentityFailure {
            code: FailureCode::PolicyUnavailable,
            message: String::from("Agent invocation price is invalid"),
        }));
    };
    auth.extra.agent_invocation_cost = Some(price);
    Ok(())
}

// _INVOCATION_COST: a finite float at least 0, from a JSON number or a numeric string; anything else is invalid
fn invocation_price(agent: &AgentResponse, billable: bool) -> Option<Decimal> {
    if !billable {
        return Some(Decimal::ZERO);
    }
    let Some(raw) = agent
        .litellm_params
        .as_ref()
        .and_then(|params| params.get("cost_per_query"))
    else {
        return Some(Decimal::ZERO);
    };
    let value: JsonData = serde_json::from_str(raw.get()).ok()?;
    match value {
        JsonData::Number(number) => {
            amount_from_text(&number.to_string()).filter(|price| *price >= Decimal::ZERO)
        }
        JsonData::String(text) => {
            amount_from_text(text.trim()).filter(|price| *price >= Decimal::ZERO)
        }
        JsonData::Null | JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => None,
    }
}
