use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::{ClaimError, JwtClaims};
use crate::jwt_claims::decode::unverified_claims;
use crate::jwt_claims::handler::JwtHandler;
use crate::jwt_claims::teams::TeamClaims as _;
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::RoleMappings;
use crate::user_roles::UserRole;

use super::claims::{SsoClaimsError, claim_groups, claim_role};
use super::roles::determine_role_from_groups;

// process_sso_jwt_access_token: the teams and role the exchange's access token adds, filling only what the identity lacks
pub fn process_sso_jwt_access_token(
    access_token: Option<&str>,
    sso_jwt_handler: Option<&JwtHandler>,
    identity: Option<OpenIdIdentity>,
    role_mappings: Option<&RoleMappings>,
    role_attribute: &str,
) -> Result<(Option<OpenIdIdentity>, Option<TypedValue>), SsoClaimsError> {
    let Some(mut updated) = identity else {
        return Ok((None, None));
    };
    // an opaque token is what a provider that issues no JWT hands back, so the identity stays as the userinfo made it
    let Some(payload) = access_token.and_then(unverified_claims) else {
        return Ok((Some(updated), None));
    };
    if let Some(handler) = sso_jwt_handler
        && updated.team_ids.is_empty()
    {
        let claims = JwtClaims {
            claims: payload.clone(),
            settings: &handler.settings,
            trusted_issuers: &handler.trusted_issuers,
        };
        updated.team_ids = claims.team_ids()?;
    }
    if updated.user_role.is_none() {
        updated.user_role = access_token_role(&payload, role_mappings, role_attribute)?;
    }
    Ok((Some(updated), Some(payload)))
}

// the role an access token adds: a configured group mapping decides first, then the direct role claim
fn access_token_role(
    payload: &TypedValue,
    role_mappings: Option<&RoleMappings>,
    role_attribute: &str,
) -> Result<Option<UserRole>, ClaimError> {
    if let Some(mappings) = role_mappings.filter(|mappings| !mappings.roles.is_empty()) {
        let groups = claim_groups(payload, &mappings.group_claim)?;
        let mapped = if groups.is_empty() {
            mappings.default_role
        } else {
            determine_role_from_groups(&groups, mappings)
        };
        if mapped.is_some() {
            return Ok(mapped);
        }
    }
    claim_role(payload, role_attribute)
}
