use super::convert::{Fields, at_least_zero, records};
use super::error::{JwtSettingsError, Location};
use super::groups::{
    AdminFields, ClaimFields, KeyFields, RbacFields, RoutingFields, ScopeFields, SyncFields,
    TeamFields, UserinfoFields, VirtualKeyFields,
};
use super::nested::{issuer, role_map, role_mapping, routing_override, scope_mapping};
use super::raw::{
    RawAdmin, RawClaims, RawKeys, RawRbac, RawRouting, RawScopes, RawSync, RawTeams, RawUserinfo,
    RawVirtualKeys, Supplied,
};
use super::records::UnregisteredClientBehavior;

pub(super) fn admin(raw: RawAdmin, fields: Fields<'_>) -> Result<AdminFields, JwtSettingsError> {
    let defaults = AdminFields::default();
    let routes = Location::from("admin_allowed_routes");
    Ok(AdminFields {
        admin_jwt_scope: fields
            .text(raw.admin_jwt_scope, "admin_jwt_scope")?
            .unwrap_or(defaults.admin_jwt_scope),
        admin_allowed_routes: records(raw.admin_allowed_routes, &routes, |text, _item| Ok(text))?
            .unwrap_or(defaults.admin_allowed_routes),
    })
}

pub(super) fn teams(raw: RawTeams, fields: Fields<'_>) -> Result<TeamFields, JwtSettingsError> {
    let defaults = TeamFields::default();
    let routes = Location::from("team_allowed_routes");
    Ok(TeamFields {
        team_id_jwt_field: fields.text(raw.team_id_jwt_field, "team_id_jwt_field")?,
        team_id_upsert: fields.flag(raw.team_id_upsert, "team_id_upsert")?,
        team_ids_jwt_field: fields.text(raw.team_ids_jwt_field, "team_ids_jwt_field")?,
        upsert_sso_user_to_team: fields
            .flag(raw.upsert_sso_user_to_team, "upsert_sso_user_to_team")?,
        team_allowed_routes: records(raw.team_allowed_routes, &routes, |text, _item| Ok(text))?
            .unwrap_or(defaults.team_allowed_routes),
        team_id_default: fields.text(raw.team_id_default, "team_id_default")?,
        team_alias_jwt_field: fields.text(raw.team_alias_jwt_field, "team_alias_jwt_field")?,
    })
}

pub(super) fn claims(raw: RawClaims, fields: Fields<'_>) -> Result<ClaimFields, JwtSettingsError> {
    let roles = Location::from("user_allowed_roles");
    Ok(ClaimFields {
        org_id_jwt_field: fields.text(raw.org_id_jwt_field, "org_id_jwt_field")?,
        org_alias_jwt_field: fields.text(raw.org_alias_jwt_field, "org_alias_jwt_field")?,
        user_id_jwt_field: fields.text(raw.user_id_jwt_field, "user_id_jwt_field")?,
        user_email_jwt_field: fields.text(raw.user_email_jwt_field, "user_email_jwt_field")?,
        user_allowed_email_domain: fields
            .text(raw.user_allowed_email_domain, "user_allowed_email_domain")?,
        user_roles_jwt_field: fields.text(raw.user_roles_jwt_field, "user_roles_jwt_field")?,
        user_allowed_roles: records(raw.user_allowed_roles, &roles, |text, _item| Ok(text))?,
        user_id_upsert: fields.flag(raw.user_id_upsert, "user_id_upsert")?,
        end_user_id_jwt_field: fields.text(raw.end_user_id_jwt_field, "end_user_id_jwt_field")?,
        agent_id_jwt_field: fields.text(raw.agent_id_jwt_field, "agent_id_jwt_field")?,
        mcp_client_id_jwt_field: fields
            .text(raw.mcp_client_id_jwt_field, "mcp_client_id_jwt_field")?,
    })
}

pub(super) fn keys(raw: RawKeys, fields: Fields<'_>) -> Result<KeyFields, JwtSettingsError> {
    let defaults = KeyFields::default();
    let public_key_ttl = fields
        .number(raw.public_key_ttl, "public_key_ttl")?
        .unwrap_or(defaults.public_key_ttl);
    let stale_ttl = fields
        .number(raw.public_key_stale_ttl, "public_key_stale_ttl")?
        .unwrap_or(defaults.public_key_stale_ttl);
    Ok(KeyFields {
        public_key_ttl,
        public_key_stale_ttl: at_least_zero(stale_ttl, &Location::from("public_key_stale_ttl"))?,
    })
}

pub(super) fn rbac(raw: RawRbac, fields: Fields<'_>) -> Result<RbacFields, JwtSettingsError> {
    let defaults = RbacFields::default();
    let routes = Location::from("public_allowed_routes");
    Ok(RbacFields {
        public_allowed_routes: records(raw.public_allowed_routes, &routes, |text, _item| Ok(text))?
            .unwrap_or(defaults.public_allowed_routes),
        enforce_rbac: fields.flag(raw.enforce_rbac, "enforce_rbac")?,
        roles_jwt_field: fields.text(raw.roles_jwt_field, "roles_jwt_field")?,
        role_mappings: records(
            raw.role_mappings,
            &Location::from("role_mappings"),
            role_mapping,
        )?,
        object_id_jwt_field: fields.text(raw.object_id_jwt_field, "object_id_jwt_field")?,
    })
}

pub(super) fn scopes(raw: RawScopes, fields: Fields<'_>) -> Result<ScopeFields, JwtSettingsError> {
    Ok(ScopeFields {
        scope_mappings: records(
            raw.scope_mappings,
            &Location::from("scope_mappings"),
            scope_mapping,
        )?,
        enforce_scope_based_access: fields
            .flag(raw.enforce_scope_based_access, "enforce_scope_based_access")?,
        enforce_team_based_model_access: fields.flag(
            raw.enforce_team_based_model_access,
            "enforce_team_based_model_access",
        )?,
    })
}

pub(super) fn sync(raw: RawSync, fields: Fields<'_>) -> Result<SyncFields, JwtSettingsError> {
    Ok(SyncFields {
        jwt_litellm_role_map: records(
            raw.jwt_litellm_role_map,
            &Location::from("jwt_litellm_role_map"),
            role_map,
        )?,
        sync_user_role_and_teams: fields
            .flag(raw.sync_user_role_and_teams, "sync_user_role_and_teams")?,
    })
}

pub(super) fn userinfo(
    raw: RawUserinfo,
    fields: Fields<'_>,
) -> Result<UserinfoFields, JwtSettingsError> {
    let defaults = UserinfoFields::default();
    Ok(UserinfoFields {
        oidc_userinfo_endpoint: fields
            .text(raw.oidc_userinfo_endpoint, "oidc_userinfo_endpoint")?,
        oidc_userinfo_enabled: fields.flag(raw.oidc_userinfo_enabled, "oidc_userinfo_enabled")?,
        oidc_userinfo_cache_ttl: fields
            .number(raw.oidc_userinfo_cache_ttl, "oidc_userinfo_cache_ttl")?
            .unwrap_or(defaults.oidc_userinfo_cache_ttl),
    })
}

// the deprecated jwt_client_id_field fills virtual_key_claim_field only when that key is absent
pub(super) fn virtual_keys(
    raw: RawVirtualKeys,
    alias: Supplied<String>,
    fields: Fields<'_>,
) -> Result<VirtualKeyFields, JwtSettingsError> {
    let defaults = VirtualKeyFields::default();
    let claim_field = if matches!(raw.virtual_key_claim_field, Supplied::Absent) {
        alias
    } else {
        raw.virtual_key_claim_field
    };
    let virtual_key_claim_field = fields.text(claim_field, "virtual_key_claim_field")?;
    let virtual_key_mapping_cache_ttl = fields
        .number(
            raw.virtual_key_mapping_cache_ttl,
            "virtual_key_mapping_cache_ttl",
        )?
        .unwrap_or(defaults.virtual_key_mapping_cache_ttl);
    let behavior = "unregistered_jwt_client_behavior";
    let unregistered_jwt_client_behavior = fields
        .text(raw.unregistered_jwt_client_behavior, behavior)?
        .map(|text| text.parse::<UnregisteredClientBehavior>())
        .transpose()
        .map_err(|_unknown| JwtSettingsError::Invalid {
            location: Location::from(behavior),
        })?
        .unwrap_or(defaults.unregistered_jwt_client_behavior);
    Ok(VirtualKeyFields {
        virtual_key_claim_field,
        virtual_key_mapping_cache_ttl,
        unregistered_jwt_client_behavior,
    })
}

pub(super) fn routing(
    raw: RawRouting,
    fields: Fields<'_>,
) -> Result<RoutingFields, JwtSettingsError> {
    Ok(RoutingFields {
        routing_overrides: records(
            raw.routing_overrides,
            &Location::from("routing_overrides"),
            routing_override,
        )?,
        team_claim_fallback: fields.flag(raw.team_claim_fallback, "team_claim_fallback")?,
        fallback_to_db_teams: fields.flag(raw.fallback_to_db_teams, "fallback_to_db_teams")?,
        issuers: records(raw.issuers, &Location::from("issuers"), issuer)?,
    })
}
