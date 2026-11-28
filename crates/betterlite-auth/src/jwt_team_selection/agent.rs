use crate::agent_identity::legacy::has_legacy_identity;
use crate::agent_policy::AgentResponse;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::identity::IdentityClaims as _;
use crate::jwt_keys::agent::AgentLookup;

use super::error::TeamSelectionError;

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
        return Err(TeamSelectionError::agent_not_registered(
            claims.settings.agent_id_jwt_field.as_deref(),
            &claim,
        ));
    };
    if bound(&agent) {
        return Err(TeamSelectionError::agent_not_registered(
            claims.settings.agent_id_jwt_field.as_deref(),
            &claim,
        ));
    }
    Ok(Some(agent.agent_id))
}

// identity_managed, a top-level identity binding, and the legacy litellm_params.identity all bind an agent
fn bound(agent: &AgentResponse) -> bool {
    agent.identity_managed
        || agent.identity.is_some()
        || has_legacy_identity(agent.litellm_params.as_ref())
}
