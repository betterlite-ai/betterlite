use std::collections::BTreeMap;

use serde_json::value::RawValue;

use crate::agent_policy::AgentResponse;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::identity::IdentityClaims as _;
use crate::jwt_keys::agent::AgentLookup;

use super::error::TeamSelectionError;

// agent_endpoints/identity.py has_legacy_identity: the key counts even when its value is null
#[must_use]
pub fn has_legacy_identity(params: Option<&BTreeMap<String, Box<RawValue>>>) -> bool {
    params.is_some_and(|fields| fields.contains_key("identity"))
}

// resolve_agent_id: the claim matches a registered id first, then a name, and only an unbound agent returns
pub fn resolve_agent_id(
    claims: &JwtClaims<'_>,
    registry: &dyn AgentLookup,
) -> Result<Option<String>, TeamSelectionError> {
    let Some(claim) = claims.agent_claim()? else {
        return Ok(None);
    };
    let agent = registry
        .get_agent_by_id(&claim)
        .or_else(|| registry.get_agent_by_name(&claim));
    let Some(agent) = agent else {
        return Err(unregistered(claims, &claim));
    };
    if bound(&agent) {
        return Err(unregistered(claims, &claim));
    }
    Ok(Some(agent.agent_id))
}

// identity_managed, a top-level identity binding, and the legacy litellm_params.identity all bind an agent
fn bound(agent: &AgentResponse) -> bool {
    agent.identity_managed
        || agent.identity.is_some()
        || has_legacy_identity(agent.litellm_params.as_ref())
}

fn unregistered(claims: &JwtClaims<'_>, claim: &str) -> TeamSelectionError {
    let field = claims.settings.agent_id_jwt_field.as_deref();
    TeamSelectionError::agent_not_registered(field, claim)
}
