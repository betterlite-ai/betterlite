use std::collections::BTreeMap;

use betterlite_config::sso_claims::ClaimAttributes;

use crate::jwt_claims::TypedValue;
use crate::jwt_claims::claims::{ClaimError, JwtClaims};
use crate::jwt_claims::handler::JwtHandler;
use crate::jwt_claims::teams::TeamClaims as _;
use crate::sso_types::identity::OpenIdIdentity;
use crate::sso_types::records::{RoleMappings, TeamMappings};
use crate::user_roles::UserRole;

use super::claims::{SsoClaimsError, claim_groups, claim_role, claim_text, navigate};
use super::roles::determine_role_from_groups;

// the configured handlers and mappings a generic provider's response is projected with
pub struct ConversionConfig<'config> {
    pub jwt_handler: &'config JwtHandler,
    pub sso_jwt_handler: Option<&'config JwtHandler>,
    pub role_mappings: Option<&'config RoleMappings>,
    pub team_mappings: Option<&'config TeamMappings>,
}

// generic_response_convertor: the OpenID identity a generic provider's userinfo, id_token and access_token map to
pub fn convert_generic_response(
    response: &TypedValue,
    config: &ConversionConfig<'_>,
    attributes: &ClaimAttributes,
) -> Result<OpenIdIdentity, SsoClaimsError> {
    let mut team_ids = Vec::new();
    if let Some(handler) = config.sso_jwt_handler {
        team_ids.extend(all_team_ids(handler, response)?);
    }
    // a DB team mapping stands in for the ordinary handler's lookup even when the claim it names adds nothing
    match config
        .team_mappings
        .and_then(|mappings| mappings.team_ids_jwt_field.as_deref())
    {
        Some(field) => team_ids.extend(mapped_team_ids(response, field)?),
        None => team_ids.extend(all_team_ids(config.jwt_handler, response)?),
    }
    Ok(OpenIdIdentity {
        id: claim_text(response, &attributes.user_id)?,
        email: normalize_email(claim_text(response, &attributes.email)?.as_deref()),
        first_name: claim_text(response, &attributes.first_name)?,
        last_name: claim_text(response, &attributes.last_name)?,
        display_name: claim_text(response, &attributes.display_name)?,
        picture: None,
        provider: claim_text(response, &attributes.provider)?,
        team_ids,
        user_role: convert_role(response, config.role_mappings, &attributes.role)?,
        extra_fields: extra_fields(response, &attributes.extra_attributes)?,
    })
}

// the SSO-specific handler's teams, which the DB mapping adds to rather than deduplicates against
fn all_team_ids(
    handler: &JwtHandler,
    response: &TypedValue,
) -> Result<Vec<String>, SsoClaimsError> {
    let claims = JwtClaims {
        claims: response.clone(),
        settings: &handler.settings,
        trusted_issuers: &handler.trusted_issuers,
    };
    Ok(claims.all_team_ids()?)
}

// the teams a DB team mapping names: a missing or non-list claim adds none, and a list holds strings
fn mapped_team_ids(response: &TypedValue, field: &str) -> Result<Vec<String>, ClaimError> {
    let Some(found) = navigate(response, field)? else {
        return Ok(Vec::new());
    };
    if !found.json.is_array() {
        return Ok(Vec::new());
    }
    serde_json::from_value::<Vec<String>>(found.json).map_err(|source| ClaimError::Type {
        claim: field.to_owned(),
        source,
    })
}

// the convertor's role: a generic or okta mapping decides, its default role standing in when no group matches
fn convert_role(
    response: &TypedValue,
    role_mappings: Option<&RoleMappings>,
    role_attribute: &str,
) -> Result<Option<UserRole>, ClaimError> {
    if let Some(mappings) = role_mappings
        && matches!(
            mappings.provider.to_lowercase().as_str(),
            "generic" | "okta"
        )
    {
        let groups = claim_groups(response, &mappings.group_claim)?;
        let mapped = if groups.is_empty() {
            mappings.default_role
        } else {
            determine_role_from_groups(&groups, mappings)
        };
        if mapped.is_some() {
            return Ok(mapped);
        }
    }
    claim_role(response, role_attribute)
}

// the nested claims GENERIC_USER_EXTRA_ATTRIBUTES names, with a null for each path the response does not follow
fn extra_fields(
    response: &TypedValue,
    names: &[String],
) -> Result<Option<BTreeMap<String, TypedValue>>, ClaimError> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut fields = BTreeMap::new();
    for name in names {
        let value = match navigate(response, name)? {
            Some(found) => found,
            None => TypedValue {
                json: serde_json::json!(null),
            },
        };
        fields.insert(name.clone(), value);
    }
    Ok(Some(fields))
}

// normalize_email: an SSO email compares case-insensitively, so the identity keeps it lowercased
#[must_use]
pub fn normalize_email(email: Option<&str>) -> Option<String> {
    email.map(str::to_lowercase)
}
