use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::JwtClaims;
use crate::jwt_claims::decode::unverified_claims;
use crate::jwt_claims::handler::JwtHandler;
use crate::jwt_claims::teams::TeamClaims as _;
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::RoleMappings;

use super::claims::SsoClaimsError;
use super::roles::resolve_role;

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
        // a mapping with no group roles says nothing here, so the token's own role claim answers
        updated.user_role = resolve_role(
            &payload,
            role_mappings.filter(|mappings| !mappings.roles.is_empty()),
            role_attribute,
        )?;
    }
    Ok((Some(updated), Some(payload)))
}
