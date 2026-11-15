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
use super::cache::ManagementObjectCache;
use super::records::{HUMAN_KIND, SSO_INTERACTIVE};

mod tests;

impl AgentIdentityStore {
    pub async fn resolve_verified_claims(
        &self,
        claims: &Payload,
    ) -> Result<Option<ManagedAgentContext>, AgentIdentityFailure> {
        let Some((issuer, tenant, client)) = claim_strings(claims) else {
            return Ok(None);
        };
        let Some(agent_id) = self.bound_agent_id(tenant, client).await? else {
            return Ok(None);
        };
        let (agent, binding) = self.usable_agent(&agent_id).await?;
        let subject = classify_agent_subject(&binding, claims, &agent.execution_mode)?;
        let user_id = match subject.kind {
            SubjectKind::Application => None,
            SubjectKind::DelegatedSubject => Some(
                self.proven_human(issuer, tenant, claims.0.get("oid"))
                    .await?,
            ),
        };
        let mode = match subject.mode {
            SubjectMode::Autonomous => ManagedAgentMode::Autonomous,
            SubjectMode::Delegated => ManagedAgentMode::Delegated,
        };
        Ok(Some(ManagedAgentContext {
            agent_id: agent.agent_id,
            binding_revision: Some(binding.revision),
            mode,
            user_id,
            subject_oid: Some(subject.oid),
        }))
    }

    // the agent row is only usable while it is enabled, identity-managed and holds an active binding
    async fn usable_agent(
        &self,
        agent_id: &str,
    ) -> Result<(AgentResponse, AgentIdentityBinding), AgentIdentityFailure> {
        let unusable = AgentIdentityFailure {
            code: FailureCode::IdentityDenied,
            message: String::from("Agent is disabled or no longer bound to an identity"),
        };
        let Some(mut agent) = self.agent(agent_id).await? else {
            return Err(unusable);
        };
        if !agent.identity_managed || !agent.enabled.0 {
            return Err(unusable);
        }
        let Some(binding) = agent.identity.take() else {
            return Err(unusable);
        };
        if !binding.active.0 {
            return Err(unusable);
        }
        Ok((agent, binding))
    }

    async fn proven_human(
        &self,
        issuer: &str,
        tenant_id: &str,
        oid: Option<&JsonData>,
    ) -> Result<String, AgentIdentityFailure> {
        let proven = self.subject(issuer, tenant_id, oid).await?;
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
}

pub async fn resolve_managed_agent(
    claims: &Payload,
    client: Option<&PostgresStore>,
    cache: Option<Arc<dyn ManagementObjectCache>>,
) -> Result<Option<ManagedAgentContext>, AgentIdentityError> {
    let Some(client) = client else {
        return Ok(None);
    };
    AgentIdentityStore::from_client(client, cache)
        .resolve_verified_claims(claims)
        .await
        .map_err(|failure| raise_identity_failure(failure, IDENTITY_DENIED_STATUS))
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
