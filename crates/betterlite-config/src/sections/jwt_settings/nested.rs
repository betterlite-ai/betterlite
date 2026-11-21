use super::convert::{forbid, ignore, records, required, value};
use super::error::{JwtSettingsError, Location};
use super::raw::{RawIssuer, RawRoleMap, RawRoleMapping, RawRoutingOverride, RawScopeMapping};
use super::records::{
    IssuerConfig, OverridePath, RoleMap, RoleMapping, RoutingOverride, ScopeMapping,
};

pub(super) fn role_mapping(
    raw: RawRoleMapping,
    location: &Location,
) -> Result<RoleMapping, JwtSettingsError> {
    let mapping = RoleMapping {
        role: required(raw.role, &location.key("role"))?,
        internal_role: required(raw.internal_role, &location.key("internal_role"))?,
    };
    ignore(&raw.extra, location);
    Ok(mapping)
}

pub(super) fn role_map(raw: RawRoleMap, location: &Location) -> Result<RoleMap, JwtSettingsError> {
    let map = RoleMap {
        jwt_role: required(raw.jwt_role, &location.key("jwt_role"))?,
        litellm_role: required(raw.litellm_role, &location.key("litellm_role"))?,
    };
    ignore(&raw.extra, location);
    Ok(map)
}

// ScopeMapping inherits models and routes from OIDCPermissions, so pydantic checks them before scope
pub(super) fn scope_mapping(
    raw: RawScopeMapping,
    location: &Location,
) -> Result<ScopeMapping, JwtSettingsError> {
    let mapping = ScopeMapping {
        models: records(raw.models, &location.key("models"), |text, _item| Ok(text))?,
        routes: records(raw.routes, &location.key("routes"), |text, _item| Ok(text))?,
        scope: required(raw.scope, &location.key("scope"))?,
    };
    forbid(&raw.extra, location)?;
    Ok(mapping)
}

pub(super) fn routing_override(
    raw: RawRoutingOverride,
    location: &Location,
) -> Result<RoutingOverride, JwtSettingsError> {
    let routing = RoutingOverride {
        iss: required(raw.iss, &location.key("iss"))?,
        client_id: value(raw.client_id, &location.key("client_id"))?,
        scope: value(raw.scope, &location.key("scope"))?,
        aud: value(raw.aud, &location.key("aud"))?,
        path: value(raw.path, &location.key("path"))?.unwrap_or(OverridePath::Oauth2),
    };
    forbid(&raw.extra, location)?;
    Ok(routing)
}

pub(super) fn issuer(
    raw: RawIssuer,
    location: &Location,
) -> Result<IssuerConfig, JwtSettingsError> {
    let config = IssuerConfig {
        issuer: required(raw.issuer, &location.key("issuer"))?,
        jwks_url: value(raw.jwks_url, &location.key("jwks_url"))?,
        audience: value(raw.audience, &location.key("audience"))?,
        disable_audience_validation: value(
            raw.disable_audience_validation,
            &location.key("disable_audience_validation"),
        )?
        .unwrap_or(false),
        user_id_jwt_field: value(raw.user_id_jwt_field, &location.key("user_id_jwt_field"))?,
        user_email_jwt_field: value(
            raw.user_email_jwt_field,
            &location.key("user_email_jwt_field"),
        )?,
        team_id_jwt_field: value(raw.team_id_jwt_field, &location.key("team_id_jwt_field"))?,
        team_ids_jwt_field: value(raw.team_ids_jwt_field, &location.key("team_ids_jwt_field"))?,
        org_id_jwt_field: value(raw.org_id_jwt_field, &location.key("org_id_jwt_field"))?,
        end_user_id_jwt_field: value(
            raw.end_user_id_jwt_field,
            &location.key("end_user_id_jwt_field"),
        )?,
        virtual_key_claim_field: value(
            raw.virtual_key_claim_field,
            &location.key("virtual_key_claim_field"),
        )?,
        unregistered_jwt_client_behavior: value(
            raw.unregistered_jwt_client_behavior,
            &location.key("unregistered_jwt_client_behavior"),
        )?,
    };
    forbid(&raw.extra, location)?;
    // the after-validator runs once every field is valid, and an empty audience still counts as set
    match (
        config.audience.is_some(),
        config.disable_audience_validation,
    ) {
        (false, false) => Err(JwtSettingsError::AudienceMissing {
            location: location.clone(),
            issuer: config.issuer,
        }),
        (true, true) => Err(JwtSettingsError::AudienceConflict {
            location: location.clone(),
            issuer: config.issuer,
        }),
        (false, true) | (true, false) => Ok(config),
    }
}
