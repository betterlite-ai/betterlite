use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::agent_identity::classification::classify_agent_subject;
use crate::agent_identity::error::{
    AgentIdentityError, IDENTITY_DENIED_STATUS, raise_identity_failure,
};
use crate::agent_policy::{
    AgentIdentityBinding, AgentResponse, ManagedAgentContext, ManagedAgentMode,
};
use crate::identity::{AgentIdentityFailure, FailureCode, SubjectKind, SubjectMode};

use super::AgentIdentityStore;
use super::agent;
use super::binding::bound_agent_id;
use super::cache::ManagementObjectCache;
use super::records::{HUMAN_KIND, SSO_INTERACTIVE};
use super::subject;

mod tests;

pub async fn resolve_verified_claims(
    store: &AgentIdentityStore,
    claims: &Payload,
) -> Result<Option<ManagedAgentContext>, AgentIdentityFailure> {
    let Some((issuer, tenant, client)) = claim_strings(claims) else {
        return Ok(None);
    };
    let Some(agent_id) = bound_agent_id(store, tenant, client).await? else {
        return Ok(None);
    };
    let (stored, binding) = usable_agent(store, &agent_id).await?;
    let agent_subject = classify_agent_subject(&binding, claims, &stored.execution_mode)?;
    let user_id = match agent_subject.kind {
        SubjectKind::Application => None,
        SubjectKind::DelegatedSubject => {
            Some(proven_human(store, issuer, tenant, claims.0.get("oid")).await?)
        }
    };
    let mode = match agent_subject.mode {
        SubjectMode::Autonomous => ManagedAgentMode::Autonomous,
        SubjectMode::Delegated => ManagedAgentMode::Delegated,
    };
    Ok(Some(ManagedAgentContext {
        agent_id: stored.agent_id,
        binding_revision: Some(binding.revision),
        mode,
        user_id,
        subject_oid: Some(agent_subject.oid),
    }))
}

pub async fn resolve_managed_agent(
    claims: &Payload,
    client: Option<&PostgresStore>,
    cache: Option<Arc<dyn ManagementObjectCache>>,
) -> Result<Option<ManagedAgentContext>, AgentIdentityError> {
    let Some(database) = client else {
        return Ok(None);
    };
    let store = AgentIdentityStore::from_client(database, cache);
    resolve_verified_claims(&store, claims)
        .await
        .map_err(|failure| raise_identity_failure(failure, IDENTITY_DENIED_STATUS))
}

// the agent row is only usable while it is enabled, identity-managed and holds an active binding
async fn usable_agent(
    store: &AgentIdentityStore,
    agent_id: &str,
) -> Result<(AgentResponse, AgentIdentityBinding), AgentIdentityFailure> {
    let unusable = AgentIdentityFailure {
        code: FailureCode::IdentityDenied,
        message: String::from("Agent is disabled or no longer bound to an identity"),
    };
    let Some(mut stored) = agent::load(store, agent_id).await? else {
        return Err(unusable);
    };
    if !stored.identity_managed || !stored.enabled.0 {
        return Err(unusable);
    }
    let Some(binding) = stored.identity.take() else {
        return Err(unusable);
    };
    if !binding.active.0 {
        return Err(unusable);
    }
    Ok((stored, binding))
}

async fn proven_human(
    store: &AgentIdentityStore,
    issuer: &str,
    tenant_id: &str,
    oid: Option<&JsonData>,
) -> Result<String, AgentIdentityFailure> {
    let proven = subject::find(store, issuer, tenant_id, oid).await?;
    let human = proven
        .filter(|row| row.kind == HUMAN_KIND && row.verified_via == SSO_INTERACTIVE)
        .and_then(|row| row.user_id);
    let Some(user_id) = human else {
        return Err(AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from(
                "The delegated user must first sign in through trusted Microsoft SSO",
            ),
        });
    };
    Ok(user_id)
}

// iss, tid and azp are the three strings LiteLLM requires before it looks any binding up
fn claim_strings(claims: &Payload) -> Option<(&str, &str, &str)> {
    Some((
        claim_text(claims, "iss")?,
        claim_text(claims, "tid")?,
        claim_text(claims, "azp")?,
    ))
}

fn claim_text<'claims>(claims: &'claims Payload, name: &str) -> Option<&'claims str> {
    match claims.0.get(name) {
        Some(JsonData::String(text)) => Some(text),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => None,
    }
}
